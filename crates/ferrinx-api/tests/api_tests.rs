//
#[path = "common/mod.rs"] mod common;

use ferrinx_common::UserRole;
use serde_json::json;

use common::TestApp;

#[tokio::test]
async fn test_health_endpoint() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{}/api/v1/health", addr))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["data"]["status"], "ok");
}

#[tokio::test]
async fn test_ready_endpoint() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{}/api/v1/ready", addr))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["data"]["database"], true);
}

#[tokio::test]
async fn test_bootstrap_creates_admin() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/bootstrap", addr))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["data"]["api_key"].is_string());

    let api_key = body["data"]["api_key"].as_str().unwrap();

    let protected_response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth(api_key)
        .send()
        .await
        .unwrap();

    assert!(protected_response.status().is_success());
}

#[tokio::test]
async fn test_bootstrap_fails_when_users_exist() {
    let test_app = TestApp::new().await;
    test_app.db.create_user("existing_user", UserRole::User).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/bootstrap", addr))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_login_success() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("testuser", UserRole::User).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/auth/login", addr))
        .json(&json!({
            "username": "testuser",
            "password": "password"
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["data"]["api_key"].is_string());
    assert_eq!(body["data"]["user_id"], user.id.to_string());
}

#[tokio::test]
async fn test_login_invalid_credentials() {
    let test_app = TestApp::new().await;
    test_app.db.create_user("testuser", UserRole::User).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/auth/login", addr))
        .json(&json!({
            "username": "testuser",
            "password": "wrong_password"
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_api_key_create() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("keyuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "initial-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "name": "new-api-key"
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["data"]["key"].is_string());
    assert_eq!(body["data"]["name"], "new-api-key");
}

#[tokio::test]
async fn test_api_key_list() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("listuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "key1", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    let keys = body["data"].as_array().unwrap();
    assert!(!keys.is_empty());
}

#[tokio::test]
async fn test_api_key_revoke() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("revokeuser", UserRole::User).await;
    let (key_id, raw_key) = test_app.db.create_api_key(&user, "key-to-revoke", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .delete(format!("http://{}/api/v1/api-keys/{}", addr, key_id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn test_model_registration() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("modeluser", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/models/register", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "name": "test-model",
            "version": "1.0.0",
            "file_path": common::lenet_model_path()
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["data"]["name"], "test-model");
    assert_eq!(body["data"]["version"], "1.0.0");
}

#[tokio::test]
async fn test_model_list() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("listmodeluser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "key", false).await;
    test_app.db.create_model("existing-model", "1.0").await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{}/api/v1/models", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    let models = body["data"].as_array().unwrap();
    assert!(!models.is_empty());
}

#[tokio::test]
async fn test_sync_inference() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("inferuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;
    let model = test_app.db.create_model("test-model", "1.0").await;
    let (addr, _handle) = test_app.start_server().await;

    // LeNet expects a 1x1x28x28 tensor (batch, channels, height, width)
    // Input name from ONNX model is "import/Placeholder:0"
    let input_data: Vec<f32> = vec![0.0; 1 * 1 * 28 * 28];

    let client = reqwest::Client::new();
    let response = client
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

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["data"]["outputs"].is_object());
    assert!(body["data"]["latency_ms"].is_number());
}

#[tokio::test]
async fn test_sync_inference_invalid_model() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("inferuser2", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/inference/sync", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "model_id": "00000000-0000-0000-0000-000000000000",
            "inputs": {
                "input": [1.0, 2.0, 3.0]
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_permission_denied_for_non_admin() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("normaluser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "user-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/admin/users", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "username": "newuser",
            "password": "password",
            "role": "user"
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_admin_can_create_user() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/admin/users", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "username": "newuser",
            "password": "password",
            "role": "user"
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn test_authentication_required() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_invalid_api_key() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{}/api/v1/api-keys", addr))
        .bearer_auth("invalid-api-key")
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_user_list_tasks() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("taskuser", UserRole::User).await;
    let (key_id, raw_key) = test_app.db.create_api_key(&user, "task-key", false).await;
    let model = test_app.db.create_model("task-model", "1.0").await;
    test_app.db.create_task(&model, &user, &key_id).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{}/api/v1/inference", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    let tasks = body["data"].as_array().unwrap();
    assert!(!tasks.is_empty());
}

#[tokio::test]
async fn test_task_cancellation() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("canceluser", UserRole::User).await;
    let (key_id, raw_key) = test_app.db.create_api_key(&user, "cancel-key", false).await;
    let model = test_app.db.create_model("cancel-model", "1.0").await;
    let task = test_app.db.create_task(&model, &user, &key_id).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .delete(format!("http://{}/api/v1/inference/{}", addr, task.id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn test_logout() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("logoutuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "logout-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/auth/logout", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

fn lenet_config_path() -> String {
    common::models_dir().join("lenet.toml").to_string_lossy().to_string()
}

fn test_image_path() -> String {
    common::models_dir().join("1.png").to_string_lossy().to_string()
}

#[tokio::test]
async fn test_model_upload_with_config() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("uploaduser", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let model_path = common::lenet_model_path();
    let config_path = lenet_config_path();

    let model_data = std::fs::read(&model_path).expect("Failed to read model file");
    let config_data = std::fs::read_to_string(&config_path).expect("Failed to read config file");

    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("name", "lenet-with-config")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text(config_data).file_name("model.toml"));

    let response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&raw_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap();
    if !status.is_success() {
        eprintln!("Upload failed with status {}: {:?}", status, body);
    }
    assert!(status.is_success());

    assert_eq!(body["data"]["name"], "lenet-with-config");
    assert_eq!(body["data"]["version"], "1.0.0");
    assert!(body["data"]["metadata"].is_object());
    assert!(body["data"]["is_valid"].as_bool().unwrap());
}

#[tokio::test]
async fn test_model_upload_without_config() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("uploaduser2", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let model_path = common::lenet_model_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");

    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("name", "lenet-no-config")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"));

    let response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&raw_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["data"]["name"], "lenet-no-config");
    assert!(body["data"]["metadata"].is_null());
    assert!(!body["data"]["is_valid"].as_bool().unwrap());
    assert!(body["data"]["validation_error"].as_str().unwrap().contains("config"));
}

#[tokio::test]
async fn test_model_upload_invalid_config() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("uploaduser3", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let model_path = common::lenet_model_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");

    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("name", "lenet-bad-config")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text("invalid toml [[[").file_name("model.toml"));

    let response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&raw_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_image_inference_with_config() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("imginferuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;

    let model_path = common::lenet_model_path();
    let config_path = lenet_config_path();

    let model_data = std::fs::read(&model_path).expect("Failed to read model file");
    let config_data = std::fs::read_to_string(&config_path).expect("Failed to read config file");

    let admin = test_app.db.create_user("imgadmin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;

    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("name", "lenet-image-test")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data.clone()).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text(config_data).file_name("model.toml"));

    let upload_response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&admin_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(upload_response.status().is_success());
    let upload_body: serde_json::Value = upload_response.json().await.unwrap();
    let model_id = upload_body["data"]["id"].as_str().unwrap();

    let image_path = test_image_path();
    let image_data = std::fs::read(&image_path).expect("Failed to read test image");

    let infer_form = reqwest::multipart::Form::new()
        .text("model_id", model_id.to_string())
        .part("image", reqwest::multipart::Part::bytes(image_data).file_name("test.png"));

    let infer_response = client
        .post(format!("http://{}/api/v1/inference/image", addr))
        .bearer_auth(&raw_key)
        .multipart(infer_form)
        .send()
        .await
        .unwrap();

    let infer_status = infer_response.status();
    let infer_body: serde_json::Value = infer_response.json().await.unwrap();
    if !infer_status.is_success() {
        eprintln!("Image inference failed with status {}: {:?}", infer_status, infer_body);
    }
    assert!(infer_status.is_success());

    assert!(infer_body["data"]["result"].is_object());
    assert!(infer_body["data"]["latency_ms"].is_number());
}

#[tokio::test]
async fn test_image_inference_by_name_version() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("nameinferuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;

    let model_path = common::lenet_model_path();
    let config_path = lenet_config_path();

    let model_data = std::fs::read(&model_path).expect("Failed to read model file");
    let config_data = std::fs::read_to_string(&config_path).expect("Failed to read config file");

    let admin = test_app.db.create_user("nameadmin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;

    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("name", "lenet-name-test")
        .text("version", "2.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text(config_data).file_name("model.toml"));

    let upload_response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&admin_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(upload_response.status().is_success());

    let image_path = test_image_path();
    let image_data = std::fs::read(&image_path).expect("Failed to read test image");

    let infer_form = reqwest::multipart::Form::new()
        .text("name", "lenet-name-test")
        .text("version", "2.0.0")
        .part("image", reqwest::multipart::Part::bytes(image_data).file_name("test.png"));

    let infer_response = client
        .post(format!("http://{}/api/v1/inference/image", addr))
        .bearer_auth(&raw_key)
        .multipart(infer_form)
        .send()
        .await
        .unwrap();

    assert!(infer_response.status().is_success());
}

#[tokio::test]
async fn test_image_inference_model_without_config() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("noconfuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;

    let model = test_app.db.create_model("model-no-config", "1.0").await;

    let (addr, _handle) = test_app.start_server().await;

    let image_path = test_image_path();
    let image_data = std::fs::read(&image_path).expect("Failed to read test image");

    let client = reqwest::Client::new();

    let infer_form = reqwest::multipart::Form::new()
        .text("model_id", model.id.to_string())
        .part("image", reqwest::multipart::Part::bytes(image_data).file_name("test.png"));

    let infer_response = client
        .post(format!("http://{}/api/v1/inference/image", addr))
        .bearer_auth(&raw_key)
        .multipart(infer_form)
        .send()
        .await
        .unwrap();

    assert!(infer_response.status().is_client_error());
}

#[tokio::test]
async fn test_image_inference_no_image() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("noimguser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;

    let model = test_app.db.create_model("model-no-img", "1.0").await;

    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let infer_form = reqwest::multipart::Form::new()
        .text("model_id", model.id.to_string());

    let infer_response = client
        .post(format!("http://{}/api/v1/inference/image", addr))
        .bearer_auth(&raw_key)
        .multipart(infer_form)
        .send()
        .await
        .unwrap();

    assert!(infer_response.status().is_client_error());
}

#[tokio::test]
async fn test_image_inference_model_not_found() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("notfounduser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;

    let (addr, _handle) = test_app.start_server().await;

    let image_path = test_image_path();
    let image_data = std::fs::read(&image_path).expect("Failed to read test image");

    let client = reqwest::Client::new();

    let infer_form = reqwest::multipart::Form::new()
        .text("model_id", "00000000-0000-0000-0000-000000000000")
        .part("image", reqwest::multipart::Part::bytes(image_data).file_name("test.png"));

    let infer_response = client
        .post(format!("http://{}/api/v1/inference/image", addr))
        .bearer_auth(&raw_key)
        .multipart(infer_form)
        .send()
        .await
        .unwrap();

    assert!(infer_response.status().is_client_error());
}

#[tokio::test]
async fn test_model_update_config() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("updateconfiguser", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;

    let model_path = common::lenet_model_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");

    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("name", "lenet-update-config")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"));

    let upload_response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&raw_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(upload_response.status().is_success());
    let upload_body: serde_json::Value = upload_response.json().await.unwrap();
    let model_id = upload_body["data"]["id"].as_str().unwrap();
    assert!(!upload_body["data"]["is_valid"].as_bool().unwrap());

    let config_toml = r#"
[[inputs]]
name = "import/Placeholder:0"
shape = [-1, 1, 28, 28]
dtype = "float32"

[[inputs.preprocess]]
type = "resize"
size = [28, 28]

[[inputs.preprocess]]
type = "grayscale"

[[inputs.preprocess]]
type = "to_tensor"
dtype = "float32"
scale = 255.0

[[outputs]]
name = "output"
shape = [-1, 10]
dtype = "float32"

[[outputs.postprocess]]
type = "argmax"
"#;

    let update_response = client
        .put(format!("http://{}/api/v1/models/{}", addr, model_id))
        .bearer_auth(&raw_key)
        .json(&json!({
            "config": config_toml
        }))
        .send()
        .await
        .unwrap();

    assert!(update_response.status().is_success());
    let update_body: serde_json::Value = update_response.json().await.unwrap();
    assert!(update_body["data"]["is_valid"].as_bool().unwrap());
    assert!(update_body["data"]["metadata"].is_object());
}

#[tokio::test]
async fn test_delete_invalid_model() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("deleteinvaliduser", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;

    let model_path = common::lenet_model_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");

    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("name", "lenet-invalid-delete")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"));

    let upload_response = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&raw_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(upload_response.status().is_success());
    let upload_body: serde_json::Value = upload_response.json().await.unwrap();
    let model_id = upload_body["data"]["id"].as_str().unwrap();
    assert!(!upload_body["data"]["is_valid"].as_bool().unwrap());

    let delete_response = client
        .delete(format!("http://{}/api/v1/models/{}", addr, model_id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(delete_response.status().is_success());

    let get_response = client
        .get(format!("http://{}/api/v1/models/{}", addr, model_id))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    assert!(get_response.status().is_client_error());
}

#[tokio::test]
async fn test_list_models_filter_by_valid() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("listvaliduser", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;

    let model_path = common::lenet_model_path();
    let config_path = lenet_config_path();
    let model_data = std::fs::read(&model_path).expect("Failed to read model file");
    let config_data = std::fs::read_to_string(&config_path).expect("Failed to read config file");

    let (addr, _handle) = test_app.start_server().await;

    let client = reqwest::Client::new();

    let form_valid = reqwest::multipart::Form::new()
        .text("name", "valid-model")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data.clone()).file_name("lenet.onnx"))
        .part("config", reqwest::multipart::Part::text(config_data).file_name("model.toml"));

    let upload_valid = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&raw_key)
        .multipart(form_valid)
        .send()
        .await
        .unwrap();
    assert!(upload_valid.status().is_success());

    let form_invalid = reqwest::multipart::Form::new()
        .text("name", "invalid-model")
        .text("version", "1.0.0")
        .part("file", reqwest::multipart::Part::bytes(model_data).file_name("lenet.onnx"));

    let upload_invalid = client
        .post(format!("http://{}/api/v1/models/upload", addr))
        .bearer_auth(&raw_key)
        .multipart(form_invalid)
        .send()
        .await
        .unwrap();
    assert!(upload_invalid.status().is_success());

    let list_valid = client
        .get(format!("http://{}/api/v1/models?is_valid=true", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    let valid_body: serde_json::Value = list_valid.json().await.unwrap();
    let valid_models = valid_body["data"].as_array().unwrap();
    assert!(valid_models.iter().all(|m| m["is_valid"].as_bool().unwrap()));

    let list_invalid = client
        .get(format!("http://{}/api/v1/models?is_valid=false", addr))
        .bearer_auth(&raw_key)
        .send()
        .await
        .unwrap();

    let invalid_body: serde_json::Value = list_invalid.json().await.unwrap();
    let invalid_models = invalid_body["data"].as_array().unwrap();
    assert!(invalid_models.iter().all(|m| !m["is_valid"].as_bool().unwrap()));
}
