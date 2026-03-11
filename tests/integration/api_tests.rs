use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use ferrinx_common::{OnnxConfig, Permissions, UserRole};
use serde_json::json;
use tempfile::NamedTempFile;
use uuid::Uuid;

mod fixtures;

use fixtures::{MockInferenceEngine, MockRedis, TestDb};

fn hash_password(password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

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
            "file_path": "/models/test.onnx"
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

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/inference/sync", addr))
        .bearer_auth(&raw_key)
        .json(&json!({
            "model_id": model.id.to_string(),
            "inputs": {
                "input": [1.0, 2.0, 3.0]
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
