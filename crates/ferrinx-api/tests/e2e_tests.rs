//
#[path = "common/mod.rs"] mod common;

use ferrinx_common::UserRole;
use serde_json::json;
use futures::future::join_all;

use common::TestApp;

#[tokio::test]
async fn test_e2e_bootstrap_and_login() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let bootstrap_response = client
        .post(format!("http://{}/api/v1/bootstrap", addr))
        .send()
        .await
        .unwrap();

    assert!(bootstrap_response.status().is_success());
    let bootstrap_body: serde_json::Value = bootstrap_response.json().await.unwrap();
    let bootstrap_key = bootstrap_body["data"]["api_key"].as_str().unwrap();

    let protected_response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth(bootstrap_key)
        .send()
        .await
        .unwrap();
    assert!(protected_response.status().is_success());

    test_app.db.create_user("login_user", UserRole::User).await;

    let login_response = client
        .post(format!("http://{}/api/v1/auth/login", addr))
        .json(&json!({
            "username": "login_user",
            "password": "password"
        }))
        .send()
        .await
        .unwrap();

    assert!(login_response.status().is_success());
    let login_body: serde_json::Value = login_response.json().await.unwrap();
    assert!(login_body["data"]["api_key"].is_string());
}

#[tokio::test]
async fn test_e2e_full_api_key_workflow() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("apikey_user", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "initial-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let create_response = client
        .post(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "name": "new-api-key",
            "permissions": {
                "models": ["read"],
                "inference": ["execute"],
                "api_keys": ["read", "write"],
                "admin": false
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(create_response.status().is_success());
    let create_body: serde_json::Value = create_response.json().await.unwrap();
    let new_key_id = create_body["data"]["id"].as_str().unwrap();

    let list_response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(list_response.status().is_success());
    let list_body: serde_json::Value = list_response.json().await.unwrap();
    let keys = list_body["data"].as_array().unwrap();
    assert!(keys.len() >= 2);

    let update_response = client
        .put(format!("http://{}/api/v1/api-keys/{}", addr, new_key_id))
        .bearer_auth(&raw_key)
        .json(&json!({
            "name": "updated-key-name"
        }))
        .send()
        .await
        .unwrap();

    assert!(update_response.status().is_success());

    let delete_response = client
        .delete(format!("http://{}/api/v1/api-keys/{}", addr, new_key_id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(delete_response.status().is_success());
}

#[tokio::test]
async fn test_e2e_model_management() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("model_user", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let register_response = client
        .post(format!("http://{}/api/v1/models/register", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "name": "e2e-test-model",
            "version": "1.0.0",
            "file_path": common::lenet_model_path()
        }))
        .send()
        .await
        .unwrap();

    assert!(register_response.status().is_success());
    let register_body: serde_json::Value = register_response.json().await.unwrap();
    let model_id = register_body["data"]["id"].as_str().unwrap();

    let list_response = client
        .get(format!("http://{}/api/v1/models", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(list_response.status().is_success());
    let list_body: serde_json::Value = list_response.json().await.unwrap();
    let models = list_body["data"].as_array().unwrap();
    assert!(models.iter().any(|m| m["name"] == "e2e-test-model"));

    let get_response = client
        .get(format!("http://{}/api/v1/models/{}", addr, model_id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(get_response.status().is_success());

    let delete_response = client
        .delete(format!("http://{}/api/v1/models/{}", addr, model_id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(delete_response.status().is_success());
}

#[tokio::test]
async fn test_e2e_sync_inference_workflow() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("infer_user", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;
    let model = test_app.db.create_model("inference-model", "1.0").await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let input_data: Vec<f32> = vec![0.0; 1 * 1 * 28 * 28];
    let infer_response = client
        .post(format!("http://{}/api/v1/inference/sync", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "model_id": model.id.to_string(),
            "inputs": {
                "import/Placeholder:0": input_data
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(infer_response.status().is_success());
    let infer_body: serde_json::Value = infer_response.json().await.unwrap();
    assert!(infer_body["data"]["outputs"].is_object());
    assert!(infer_body["data"]["latency_ms"].is_number());
}

#[tokio::test]
async fn test_e2e_admin_user_management() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin_user", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let create_user_response = client
        .post(format!("http://{}/api/v1/admin/users", addr))
        .bearer_auth(&admin_key)
        .json(&json!({
            "username": "managed_user",
            "password": "secure_password",
            "role": "user"
        }))
        .send()
        .await
        .unwrap();

    assert!(create_user_response.status().is_success());

    let list_users_response = client
        .get(format!("http://{}/api/v1/admin/users", addr))
        .bearer_auth(&admin_key)
        .send()
        .await
        .unwrap();

    assert!(list_users_response.status().is_success());
    let list_body: serde_json::Value = list_users_response.json().await.unwrap();
    let users = list_body["data"].as_array().unwrap();
    assert!(users.iter().any(|u| u["username"] == "managed_user"));
}

#[tokio::test]
async fn test_e2e_task_lifecycle() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("task_user", UserRole::User).await;
    let (key_id, raw_key) = test_app.db.create_api_key(&user, "task-key", false).await;
    let model = test_app.db.create_model("task-model", "1.0").await;
    let task = test_app.db.create_task(&model, &user, &key_id).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let list_response = client
        .get(format!("http://{}/api/v1/inference", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(list_response.status().is_success());
    let list_body: serde_json::Value = list_response.json().await.unwrap();
    assert!(!list_body["data"].as_array().unwrap().is_empty());

    let get_response = client
        .get(format!("http://{}/api/v1/inference/{}", addr, task.id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(get_response.status().is_success());
    let get_body: serde_json::Value = get_response.json().await.unwrap();
    assert_eq!(get_body["data"]["task_id"], task.id.to_string());

    let cancel_response = client
        .delete(format!("http://{}/api/v1/inference/{}", addr, task.id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(cancel_response.status().is_success());
}

#[tokio::test]
async fn test_e2e_permission_isolation() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("iso_admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;

    let user = test_app.db.create_user("iso_user", UserRole::User).await;
    let (_, user_key) = test_app.db.create_api_key(&user, "user-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let admin_access = client
        .post(format!("http://{}/api/v1/admin/users", addr))
        .bearer_auth(&admin_key)
        .json(&json!({
            "username": "new_user",
            "password": "password"
        }))
        .send()
        .await
        .unwrap();

    assert!(admin_access.status().is_success());

    let user_access = client
        .post(format!("http://{}/api/v1/admin/users", addr))
        .bearer_auth(&user_key)
        .json(&json!({
            "username": "should_fail",
            "password": "password"
        }))
        .send()
        .await
        .unwrap();

    assert!(user_access.status().is_client_error());
}

#[tokio::test]
async fn test_e2e_health_check_resilience() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    for _ in 0..5 {
        let response = client
            .get(format!("http://{}/api/v1/health", addr))
            .send()
            .await
            .unwrap();

        assert!(response.status().is_success());
    }

    let ready_response = client
        .get(format!("http://{}/api/v1/ready", addr))
        .send()
        .await
        .unwrap();

    assert!(ready_response.status().is_success());
    let ready_body: serde_json::Value = ready_response.json().await.unwrap();
    assert!(ready_body["data"]["database"].as_bool().unwrap());
}

#[tokio::test]
async fn test_e2e_concurrent_requests() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("concurrent_user", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "concurrent-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let mut handles = vec![];

    for _ in 0..10 {
        let url = format!("http://{}/api/v1/api-keys", addr);
        let key = raw_key.clone();
        let handle = tokio::spawn(async move {
            let client = reqwest::Client::new();
            client
                .get(&url)
                .bearer_auth(&key)
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        });
        handles.push(handle);
    }

    let results: Vec<_> = join_all(handles).await;
    for result in results {
        assert!(result.unwrap());
    }
}

#[tokio::test]
async fn test_e2e_error_responses() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let no_auth_response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .send()
        .await
        .unwrap();

    assert!(no_auth_response.status().is_client_error());
    let body: serde_json::Value = no_auth_response.json().await.unwrap();
    assert!(body["error"].is_object());

    let invalid_key_response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth("invalid-key")
        .send()
        .await
        .unwrap();

    assert!(invalid_key_response.status().is_client_error());
}

#[tokio::test]
async fn test_e2e_complete_workflow() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let bootstrap: serde_json::Value = client
        .post(format!("http://{}/api/v1/bootstrap", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let admin_key = bootstrap["data"]["api_key"].as_str().unwrap();

    let model_path = common::lenet_model_path();
    let config_path = lenet_config_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");
    let config_data = std::fs::read_to_string(&config_path).expect("Failed to read config file");

    let form = reqwest::multipart::Form::new()
        .text("name", "workflow-model")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text(config_data).file_name("model.toml"));

    let model: serde_json::Value = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(admin_key)
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let model_id = model["data"]["id"].as_str().unwrap();

    let input_data: Vec<f32> = vec![0.0; 1 * 1 * 28 * 28];
    let infer: serde_json::Value = client
        .post(format!("http://{}/api/v1/inference/sync", addr))
        .bearer_auth(admin_key)
        .json(&json!({
            "model_id": model_id,
            "inputs": {"import/Placeholder:0": input_data}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(infer["data"]["outputs"].is_object());

    let api_key: serde_json::Value = client
        .post(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth(admin_key)
        .json(&json!({"name": "workflow-key"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(api_key["data"]["key"].is_string());
}

fn lenet_config_path() -> String {
    common::models_dir().join("lenet.toml").to_string_lossy().to_string()
}

fn test_image_path() -> String {
    common::models_dir().join("1.png").to_string_lossy().to_string()
}

#[tokio::test]
async fn test_e2e_model_upload_with_config_workflow() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let bootstrap: serde_json::Value = client
        .post(format!("http://{}/api/v1/bootstrap", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let admin_key = bootstrap["data"]["api_key"].as_str().unwrap();

    let model_path = common::lenet_model_path();
    let config_path = lenet_config_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");
    let config_data = std::fs::read_to_string(&config_path).expect("Failed to read config file");

    let form = reqwest::multipart::Form::new()
        .text("name", "e2e-lenet")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text(config_data).file_name("model.toml"));

    let upload_response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(admin_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(upload_response.status().is_success());
    let upload_body: serde_json::Value = upload_response.json().await.unwrap();
    assert!(upload_body["data"]["metadata"].is_object());
    let model_id = upload_body["data"]["id"].as_str().unwrap();

    let get_response = client
        .get(format!("http://{}/api/v1/models/{}", addr, model_id))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();

    assert!(get_response.status().is_success());
    let get_body: serde_json::Value = get_response.json().await.unwrap();
    assert!(get_body["data"]["metadata"].is_object());
}

#[tokio::test]
async fn test_e2e_image_inference_workflow() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let bootstrap: serde_json::Value = client
        .post(format!("http://{}/api/v1/bootstrap", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let admin_key = bootstrap["data"]["api_key"].as_str().unwrap();

    let model_path = common::lenet_model_path();
    let config_path = lenet_config_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");
    let config_data = std::fs::read_to_string(&config_path).expect("Failed to read config file");

    let form = reqwest::multipart::Form::new()
        .text("name", "e2e-lenet-img")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text(config_data).file_name("model.toml"));

    let upload_response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(admin_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(upload_response.status().is_success());

    let image_path = test_image_path();
    let image_data = std::fs::read(&image_path).expect("Failed to read test image");

    let infer_form = reqwest::multipart::Form::new()
        .text("name", "e2e-lenet-img")
        .text("version", "1.0.0")
        .part("image", reqwest::multipart::Part::bytes(image_data).file_name("test.png"));

    let infer_response = client
        .post(format!("http://{}/api/v1/inference/image", addr))
        .bearer_auth(admin_key)
        .multipart(infer_form)
        .send()
        .await
        .unwrap();

    assert!(infer_response.status().is_success());
    let infer_body: serde_json::Value = infer_response.json().await.unwrap();
    assert!(infer_body["data"]["result"].is_object());
    assert!(infer_body["data"]["latency_ms"].is_number());
}

#[tokio::test]
async fn test_e2e_model_registration_with_metadata() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let bootstrap: serde_json::Value = client
        .post(format!("http://{}/api/v1/bootstrap", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let admin_key = bootstrap["data"]["api_key"].as_str().unwrap();

    let config_toml = r#"
[[inputs]]
name = "input"
shape = [-1, 1, 28, 28]
dtype = "float32"

[[inputs.preprocess]]
type = "resize"
size = [28, 28]

[[outputs]]
name = "output"
shape = [-1, 10]
dtype = "float32"
"#;

    let config: ferrinx_core::model::config::ModelConfig = 
        ferrinx_core::model::config::ModelConfig::from_toml(config_toml).unwrap();
    let metadata = serde_json::to_value(config).unwrap();

    let register_response = client
        .post(format!("http://{}/api/v1/models/register", addr))
        .bearer_auth(admin_key)
        .json(&json!({
            "name": "model-with-metadata",
            "version": "1.0.0",
            "file_path": common::lenet_model_path(),
            "metadata": metadata
        }))
        .send()
        .await
        .unwrap();

    assert!(register_response.status().is_success());
    let register_body: serde_json::Value = register_response.json().await.unwrap();
    assert!(register_body["data"]["metadata"].is_object());
}
