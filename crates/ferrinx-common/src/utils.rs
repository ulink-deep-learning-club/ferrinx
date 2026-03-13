use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn sha256_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Hash a password using bcrypt with default cost factor (12)
pub fn hash_password(password: &str) -> Result<String, bcrypt::BcryptError> {
    bcrypt::hash(password, bcrypt::DEFAULT_COST)
}

/// Verify a password against a bcrypt hash
pub fn verify_password(password: &str, hash: &str) -> Result<bool, bcrypt::BcryptError> {
    bcrypt::verify(password, hash)
}

/// Generate a secure random password for bootstrap/admin purposes
pub fn generate_secure_password(length: usize) -> String {
    use rand::RngExt;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                            abcdefghijklmnopqrstuvwxyz\
                            0123456789\
                            !@#$%^&*";
    let mut rng = rand::rng();
    (0..length)
        .map(|_| {
            let idx: usize = rng.random::<u8>() as usize % CHARSET.len();
            CHARSET[idx] as char
        })
        .collect()
}

pub fn hash_key(key: &str) -> String {
    sha256_hash(key)
}

pub fn generate_api_key(prefix: &str) -> String {
    let random_bytes: String = (0..32)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    format!("{}_{}", prefix, random_bytes)
}

pub fn generate_uuid() -> Uuid {
    Uuid::new_v4()
}

pub fn expand_env_vars(input: &str) -> String {
    shellexpand::env(input)
        .unwrap_or_else(|_| input.into())
        .to_string()
}

pub fn validate_api_key_format(key: &str, prefix: &str) -> bool {
    key.starts_with(prefix) && key.len() > prefix.len() + 1
}

pub fn generate_request_id() -> String {
    format!("req-{}", Uuid::new_v4())
}

pub fn parse_uuid(s: &str) -> Result<Uuid, crate::error::CommonError> {
    Uuid::parse_str(s).map_err(|e| crate::error::CommonError::InvalidUuid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hash() {
        let input = "test_api_key";
        let hash = sha256_hash(input);
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, input);
    }

    #[test]
    fn test_sha256_hash_consistency() {
        let input = "test_api_key";
        let hash1 = sha256_hash(input);
        let hash2 = sha256_hash(input);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_generate_api_key() {
        let key = generate_api_key("frx_sk");
        assert!(key.starts_with("frx_sk_"));
        assert!(key.len() > 10);
    }

    #[test]
    fn test_generate_api_key_uniqueness() {
        let key1 = generate_api_key("frx_sk");
        let key2 = generate_api_key("frx_sk");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_generate_uuid() {
        let uuid = generate_uuid();
        assert!(!uuid.is_nil());
    }

    #[test]
    fn test_validate_api_key_format() {
        assert!(validate_api_key_format("frx_sk_abc123", "frx_sk"));
        assert!(!validate_api_key_format("frx_sk", "frx_sk"));
        assert!(!validate_api_key_format("wrong_prefix_abc", "frx_sk"));
    }

    #[test]
    fn test_generate_request_id() {
        let id = generate_request_id();
        assert!(id.starts_with("req-"));
    }

    #[test]
    fn test_parse_uuid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let result = parse_uuid(uuid_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_uuid_invalid() {
        let result = parse_uuid("invalid-uuid");
        assert!(result.is_err());
    }
}
