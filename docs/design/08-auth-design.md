# 认证授权设计

## 1. 认证授权架构

Ferrinx 采用基于 API Key 的认证机制和 RBAC（基于角色的访问控制）授权模型。

### 1.1 认证流程

```
Client → API Gateway → Middleware → API Handler
         ↓
    提取 API Key
         ↓
    验证 API Key
    (Redis Cache → DB Fallback)
         ↓
    提取用户信息和权限
         ↓
    注入到请求上下文
```

### 1.2 授权流程

```
API Handler → Permission Check → Business Logic
              ↓
         检查用户权限
         (基于角色 + 自定义权限)
              ↓
         允许/拒绝
```

## 2. API Key 设计

### 2.1 API Key 格式

```
frx_sk_<random_32_bytes_hex>

示例：
frx_sk_a3b8f2e1d4c5a6b7e8f9d0c1b2a3e4f5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0

临时 Key：
frx_sk_temp_<random_32_bytes_hex>
```

**设计要点**：
- 前缀 `frx_sk` 标识 Ferrinx Secret Key
- 32 字节随机数（64 个十六进制字符）
- 总长度：6 + 1 + 64 = 71 字符

### 2.2 API Key 生成

```rust
pub fn generate_api_key(prefix: &str) -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: [u8; 32] = rng.gen();
    
    let hex: String = random_bytes.iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    
    format!("{}_{}", prefix, hex)
}

pub fn generate_permanent_key() -> String {
    generate_api_key("frx_sk")
}

pub fn generate_temporary_key() -> String {
    generate_api_key("frx_sk_temp")
}
```

### 2.3 API Key 存储

数据库存储 **SHA-256 哈希**，不存储明文：

```rust
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

**数据库字段**：
```sql
key_hash VARCHAR(64) UNIQUE NOT NULL  -- SHA-256 哈希（64 个十六进制字符）
```

### 2.4 API Key 验证流程

```rust
// src/middleware/auth.rs

pub async fn validate_api_key(
    key: &str,
    state: &AppState,
) -> Result<ApiKeyInfo, ApiError> {
    // 1. 验证格式
    if !validate_api_key_format(key, "frx_sk") {
        return Err(ApiError::InvalidApiKeyFormat);
    }
    
    // 2. 计算哈希
    let key_hash = sha256_hash(key);
    
    // 3. 尝试从 Redis 获取
    if let Some(ref redis) = state.redis {
        if let Ok(Some(info)) = redis.get_api_key(&key_hash).await {
            // 4. 检查状态
            if !info.is_active {
                return Err(ApiError::InvalidApiKey);
            }
            
            // 5. 检查过期
            if is_expired(&info) {
                return Err(ApiError::InvalidApiKey);
            }
            
            return Ok(info);
        }
    }
    
    // 6. Redis 失败或未命中，降级到数据库
    warn!("Redis unavailable or cache miss, falling back to database");
    
    if let Some(record) = state.db.api_keys.find_by_hash(&key_hash).await? {
        let info = ApiKeyInfo::from(record);
        
        if !info.is_active || is_expired(&info) {
            return Err(ApiError::InvalidApiKey);
        }
        
        // 7. 异步更新 Redis 缓存
        if let Some(ref redis) = state.redis {
            let redis_clone = redis.clone();
            let info_clone = info.clone();
            tokio::spawn(async move {
                let _ = redis_clone.set_api_key(&info_clone).await;
            });
        }
        
        // 8. 异步更新 last_used_at
        let db = state.db.clone();
        let key_id = info.id.clone();
        tokio::spawn(async move {
            let _ = db.api_keys.update_last_used(&key_id).await;
        });
        
        return Ok(info);
    }
    
    Err(ApiError::InvalidApiKey)
}

fn is_expired(info: &ApiKeyInfo) -> bool {
    info.expires_at.map_or(false, |exp| Utc::now() > exp)
}
```

## 3. 权限模型

### 3.1 RBAC 角色

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    User,
    Admin,
}
```

### 3.2 权限定义

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Permissions {
    #[serde(default)]
    pub models: Vec<String>,      // ["read", "write", "delete"]
    #[serde(default)]
    pub inference: Vec<String>,   // ["execute"]
    #[serde(default)]
    pub api_keys: Vec<String>,    // ["read", "write", "delete"]
    #[serde(default)]
    pub admin: bool,
}

impl Permissions {
    /// 普通用户默认权限
    pub fn user_default() -> Self {
        Self {
            models: vec!["read".to_string()],
            inference: vec!["execute".to_string()],
            api_keys: vec!["read".to_string(), "write".to_string()],
            admin: false,
        }
    }
    
    /// 管理员默认权限
    pub fn admin_default() -> Self {
        Self {
            models: vec!["read".to_string(), "write".to_string(), "delete".to_string()],
            inference: vec!["execute".to_string()],
            api_keys: vec!["read".to_string(), "write".to_string(), "delete".to_string()],
            admin: true,
        }
    }
    
    /// 检查是否有指定权限
    pub fn has_permission(&self, resource: &str, action: &str) -> bool {
        if self.admin {
            return true;
        }
        
        match resource {
            "models" => self.models.contains(&action.to_string()),
            "inference" => self.inference.contains(&action.to_string()),
            "api_keys" => self.api_keys.contains(&action.to_string()),
            _ => false,
        }
    }
}
```

### 3.3 权限检查

```rust
// src/middleware/auth.rs

pub fn check_permission(
    api_key: &ApiKeyInfo,
    path: &str,
    method: &Method,
) -> bool {
    // Admin 拥有所有权限
    if api_key.permissions.admin {
        return true;
    }
    
    // 根据路径和方法检查权限
    match path {
        // 管理员路径
        p if p.starts_with("/api/v1/admin") => {
            false
        }
        
        // 模型管理
        p if p.starts_with("/api/v1/models") => {
            match method {
                Method::GET => api_key.permissions.has_permission("models", "read"),
                Method::POST => api_key.permissions.has_permission("models", "write"),
                Method::DELETE => api_key.permissions.has_permission("models", "delete"),
                _ => false,
            }
        }
        
        // 推理
        p if p.starts_with("/api/v1/inference") => {
            api_key.permissions.has_permission("inference", "execute")
        }
        
        // API Key 管理
        p if p.starts_with("/api/v1/api-keys") => {
            match method {
                Method::GET => api_key.permissions.has_permission("api_keys", "read"),
                Method::POST => api_key.permissions.has_permission("api_keys", "write"),
                Method::DELETE => api_key.permissions.has_permission("api_keys", "delete"),
                _ => false,
            }
        }
        
        // 其他路径默认允许
        _ => true,
    }
}
```

## 4. 用户管理

### 4.1 用户创建流程

#### Bootstrap 流程（首次初始化）

```rust
// src/handlers/bootstrap.rs

/// 生成安全随机密码
fn generate_secure_password() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                            abcdefghijklmnopqrstuvwxyz\
                            0123456789!@#$%^&*";
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

pub async fn bootstrap(
    State(state): State<AppState>,
    Json(req): Json<BootstrapRequest>,
) -> Result<Json<ApiResponse<BootstrapResponse>>, ApiError> {
    // 1. 检查 users 表是否为空
    let user_count = state.db.users.count().await?;
    if user_count > 0 {
        return Err(ApiError::BootstrapDisabled);
    }
    
    // 2. 生成安全随机密码
    let password = generate_secure_password();
    let password_hash = hash_password(&password)?;
    
    // 3. 创建第一个管理员
    let user_id = uuid::Uuid::new_v4();
    
    let user = User {
        id: user_id,
        username: req.username.clone(),
        role: UserRole::Admin,
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    state.db.users.save(&user).await?;
    
    // 4. 创建管理员 API Key
    let api_key = generate_permanent_key();
    let key_hash = sha256_hash(&api_key);
    
    let key_record = ApiKeyRecord {
        id: uuid::Uuid::new_v4(),
        user_id: user.id,
        key_hash,
        name: "Bootstrap Admin Key".to_string(),
        permissions: Permissions::admin_default(),
        is_active: true,
        is_temporary: false,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    state.db.api_keys.save(&key_record).await?;
    
    // 5. 记录安全警告
    warn!("SECURITY WARNING: Bootstrap completed with auto-generated password");
    info!("Bootstrap completed: admin user '{}' created", req.username);
    
    Ok(Json(ApiResponse::success(BootstrapResponse {
        user_id: user.id.to_string(),
        username: user.username,
        role: "admin".to_string(),
        api_key,
        password,  // 返回自动生成的密码（仅显示一次）
        message: "Bootstrap completed. Save the password securely - it will not be shown again.".to_string(),
    })))
}
```

**安全引导密码生成说明**：

Bootstrap 流程不再接受用户提供的密码，而是自动生成安全的随机密码：

1. **密码生成**：使用加密安全的随机数生成器生成 32 位字符的密码
2. **字符集**：包含大小写字母、数字和特殊字符
3. **密码哈希**：使用 bcrypt 对生成的密码进行哈希存储
4. **一次性显示**：密码仅在 bootstrap 响应中返回一次，之后不再显示
5. **安全警告**：系统记录安全警告日志，提醒管理员保存密码
6. **bcrypt 哈希**：使用 bcrypt::DEFAULT_COST（默认 12）进行密码哈希，自动处理加盐

#### 管理员创建用户

```rust
pub async fn create_user(
    State(state): State<AppState>,
    Extension(admin): Extension<ApiKeyInfo>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserResponse>>, ApiError> {
    // 1. 验证管理员权限
    if !admin.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }
    
    // 2. 检查用户名是否已存在
    if state.db.users.find_by_username(&req.username).await?.is_some() {
        return Err(ApiError::UserAlreadyExists);
    }
    
    // 3. 创建用户
    let user_id = uuid::Uuid::new_v4();
    let password_hash = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST)?;
    let role = match req.role.as_str() {
        "admin" => UserRole::Admin,
        _ => UserRole::User,
    };
    
    let user = User {
        id: user_id,
        username: req.username.clone(),
        role,
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    state.db.users.save(&user).await?;
    
    Ok(Json(ApiResponse::success(UserResponse::from(user))))
}
```

### 4.2 用户登录流程

```rust
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, ApiError> {
    // 1. 查找用户
    let user = state.db.users
        .find_by_username(&req.username)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;
    
    // 2. 验证密码
    let password_hash = /* 从数据库获取 */;
    if !bcrypt::verify(&req.password, &password_hash)? {
        return Err(ApiError::InvalidCredentials);
    }
    
    // 3. 检查用户是否激活
    if !user.is_active {
        return Err(ApiError::UserInactive);
    }
    
    // 4. 创建临时 API Key
    let temp_key = generate_temporary_key();
    let key_hash = sha256_hash(&temp_key);
    let expires_at = Utc::now() + chrono::Duration::hours(1);
    
    let key_record = ApiKeyRecord {
        id: uuid::Uuid::new_v4(),
        user_id: user.id,
        key_hash,
        name: "Temporary Key".to_string(),
        permissions: match user.role {
            UserRole::Admin => Permissions::admin_default(),
            UserRole::User => Permissions::user_default(),
        },
        is_active: true,
        is_temporary: true,
        last_used_at: None,
        expires_at: Some(expires_at),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    state.db.api_keys.save(&key_record).await?;
    
    Ok(Json(ApiResponse::success(LoginResponse {
        api_key: temp_key,
        expires_at: expires_at.to_rfc3339(),
        user: UserInfo::from(user),
    })))
}
```

## 5. API Key 管理

### 5.1 创建 API Key

```rust
pub async fn create_api_key(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiResponse<CreateApiKeyResponse>>, ApiError> {
    // 1. 检查用户已有的 API Key 数量
    let existing_keys = state.db.api_keys
        .find_by_user(&api_key.user_id)
        .await?;
    
    if existing_keys.len() >= state.config.auth.max_keys_per_user {
        return Err(ApiError::TooManyApiKeys);
    }
    
    // 2. 生成新的 API Key
    let new_key = generate_permanent_key();
    let key_hash = sha256_hash(&new_key);
    
    // 3. 解析权限
    let permissions = if let Some(perms_json) = req.permissions {
        serde_json::from_value(perms_json)?
    } else {
        // 使用默认权限
        if api_key.permissions.admin {
            Permissions::admin_default()
        } else {
            Permissions::user_default()
        }
    };
    
    // 4. 计算过期时间
    let expires_at = req.expires_days
        .map(|days| Utc::now() + chrono::Duration::days(days as i64));
    
    // 5. 保存到数据库
    let key_record = ApiKeyRecord {
        id: uuid::Uuid::new_v4(),
        user_id: api_key.user_id,
        key_hash,
        name: req.name,
        permissions,
        is_active: true,
        is_temporary: false,
        last_used_at: None,
        expires_at,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    state.db.api_keys.save(&key_record).await?;
    
    Ok(Json(ApiResponse::success(CreateApiKeyResponse {
        id: key_record.id.to_string(),
        api_key: new_key,
        name: key_record.name,
        expires_at: expires_at.map(|t| t.to_rfc3339()),
    })))
}
```

### 5.2 撤销 API Key

```rust
pub async fn revoke_api_key(
    State(state): State<AppState>,
    Extension(api_key): Extension<ApiKeyInfo>,
    Path(key_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let key_id = uuid::Uuid::parse_str(&key_id)?;
    
    // 查找要撤销的 Key
    let target_key = state.db.api_keys
        .find_by_id(&key_id)
        .await?
        .ok_or(ApiError::ApiKeyNotFound)?;
    
    // 验证权限
    if target_key.user_id != api_key.user_id && !api_key.permissions.admin {
        return Err(ApiError::PermissionDenied);
    }
    
    // 停用 Key
    state.db.api_keys.deactivate(&key_id).await?;
    
    // 删除 Redis 缓存
    if let Some(ref redis) = state.redis {
        redis.delete_api_key(&target_key.key_hash).await?;
    }
    
    Ok(Json(ApiResponse::success(())))
}
```

## 6. 安全考虑

### 6.1 API Key 安全

- **不存储明文**：数据库只存储 SHA-256 哈希
- **仅显示一次**：创建时返回明文，之后不再显示
- **传输加密**：HTTPS 传输
- **有效期控制**：支持过期时间

### 6.2 密码安全

- **bcrypt 哈希**：使用 bcrypt 算法
- **加盐存储**：bcrypt 自动加盐
- **强度可配置**：DEFAULT_COST (12)

### 6.3 防止暴力破解

- **限流保护**：基于 API Key 的限流
- **登录限流**：基于 IP 的登录限流
- **失败锁定**：连续失败后临时锁定

### 6.4 审计日志

- **记录所有关键操作**：创建用户、创建 API Key、推理请求
- **包含 Request ID**：便于追踪
- **记录时间戳和用户**：便于审计

## 7. 设计要点

### 7.1 无状态认证

- API Key 包含所有必要信息
- 不依赖 Session
- 便于水平扩展

### 7.2 缓存优化

- Redis 缓存 API Key 信息
- 减少数据库查询
- 降级到数据库保证可用性

### 7.3 权限灵活

- 基于角色的默认权限
- 支持自定义权限
- 细粒度控制

### 7.4 安全第一

- 哈希存储
- HTTPS 传输
- 有效期控制
- 审计日志
