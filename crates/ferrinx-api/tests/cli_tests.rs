//
#[path = "common/mod.rs"] mod common;

use std::io::Write;
use tempfile::NamedTempFile;

use common::TestApp;
use ferrinx_common::UserRole;

fn create_test_config_file(api_url: &str, api_key: Option<&str>) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    let content = if let Some(key) = api_key {
        format!(
            r#"
api_url = "{}"
api_key = "{}"
timeout = 30
output_format = "json"
"#,
            api_url, key
        )
    } else {
        format!(
            r#"
api_url = "{}"
timeout = 30
output_format = "json"
"#,
            api_url
        )
    };
    file.write_all(content.as_bytes()).unwrap();
    file
}

#[tokio::test]
async fn test_cli_config_load() {
    let temp_file = create_test_config_file("http://localhost:8080/api/v1", Some("test-key"));
    let path = temp_file.path().to_str().unwrap();

    let config = ferrinx_cli::config::CliConfig::load(Some(path)).unwrap();
    assert_eq!(config.api_url, "http://localhost:8080/api/v1");
    assert_eq!(config.api_key, Some("test-key".to_string()));
}

#[tokio::test]
async fn test_cli_config_default() {
    let config = ferrinx_cli::config::CliConfig::load(None::<&str>).unwrap();
    assert!(!config.api_url.is_empty());
}

#[tokio::test]
async fn test_http_client_get() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("httpuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "http-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: Some(raw_key),
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let result: serde_json::Value = client.get("/api/v1/health").await.unwrap();
    assert_eq!(result["status"], "ok");
}

#[tokio::test]
async fn test_http_client_post() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: None,
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let result: serde_json::Value = client
        .post("/api/v1/bootstrap", &serde_json::json!({}))
        .await
        .unwrap();

    assert!(result["api_key"].is_string());
}

#[tokio::test]
async fn test_http_client_delete() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("deluser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "main-key", false).await;
    let (key_to_delete, _) = test_app.db.create_api_key(&user, "delete-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: Some(raw_key.clone()),
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let list_before: serde_json::Value = client.get("/api/v1/api-keys").await.unwrap();
    let count_before = list_before.as_array().unwrap().len();

    let _ = client
        .delete::<serde_json::Value>(&format!("/api/v1/api-keys/{}", key_to_delete))
        .await;

    let list_after: serde_json::Value = client.get("/api/v1/api-keys").await.unwrap();
    let count_after = list_after.as_array().unwrap().len();

    assert!(count_after < count_before);
}

#[tokio::test]
async fn test_http_client_error_handling() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: Some("invalid-key".to_string()),
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let result = client.get::<serde_json::Value>("/api/v1/api-keys").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_parse_input_json() {
    let input = r#"{"key": "value", "number": 42}"#;
    let result = ferrinx_cli::commands::parse_input(input).unwrap();
    assert_eq!(result["key"], "value");
    assert_eq!(result["number"], 42);
}

#[tokio::test]
async fn test_parse_input_file() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, r#"{{"file_key": "file_value"}}"#).unwrap();

    let path = temp_file.path().to_str().unwrap();
    let result = ferrinx_cli::commands::parse_input(path).unwrap();
    assert_eq!(result["file_key"], "file_value");
}

#[tokio::test]
async fn test_parse_permissions() {
    let input = r#"{"admin": true, "models": ["read", "write"]}"#;
    let result = ferrinx_cli::commands::parse_permissions(input).unwrap();
    assert!(result.admin);
    assert!(result.models.contains(&"read".to_string()));
    assert!(result.models.contains(&"write".to_string()));
}

#[tokio::test]
async fn test_cli_error_display() {
    let error = ferrinx_cli::error::CliError::ApiError {
        code: "TEST_ERROR".to_string(),
        message: "Test error message".to_string(),
    };
    let display = format!("{}", error);
    assert!(display.contains("TEST_ERROR"));
    assert!(display.contains("Test error message"));
}

#[tokio::test]
async fn test_cli_http_error() {
    let error = ferrinx_cli::error::CliError::HttpError {
        status: 404,
        message: "Not Found".to_string(),
    };
    let display = format!("{}", error);
    assert!(display.contains("404"));
    assert!(display.contains("Not Found"));
}

#[tokio::test]
async fn test_bootstrap_command_flow() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: None,
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let result: serde_json::Value = client
        .post("/api/v1/bootstrap", &serde_json::json!({}))
        .await
        .unwrap();

    assert!(result["api_key"].is_string());
    assert!(result["user_id"].is_string());
}

#[tokio::test]
async fn test_api_key_operations() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("opsuser", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "ops-key", false).await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: Some(raw_key.clone()),
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let create_result: serde_json::Value = client
        .post(
            "/api/v1/api-keys",
            &serde_json::json!({
                "name": "test-key-via-cli"
            }),
        )
        .await
        .unwrap();

    assert!(create_result["key"].is_string());

    let list_result: serde_json::Value = client.get("/api/v1/api-keys").await.unwrap();
    assert!(list_result.as_array().unwrap().len() >= 1);
}

#[tokio::test]
async fn test_model_operations_via_client() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("modelops", UserRole::Admin).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: Some(raw_key.clone()),
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let register_result: serde_json::Value = client
        .post(
            "/api/v1/models/register",
            &serde_json::json!({
                "name": "cli-test-model",
                "version": "1.0.0",
                "file_path": common::lenet_model_path()
            }),
        )
        .await
        .unwrap();

    assert!(register_result["id"].is_string());

    let list_result: serde_json::Value = client.get("/api/v1/models").await.unwrap();
    assert!(!list_result.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_inference_via_client() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("inferops", UserRole::User).await;
    let (_, raw_key) = test_app.db.create_api_key(&user, "infer-key", false).await;
    let model = test_app.db.create_model("client-model", "1.0").await;
    let (addr, _handle) = test_app.start_server().await;

    let config = ferrinx_cli::config::CliConfig {
        api_url: format!("http://{}", addr),
        api_key: Some(raw_key.clone()),
        timeout: 30,
        verify_ssl: true,
        output_format: ferrinx_cli::config::OutputFormat::Json,
    };

    let client = ferrinx_cli::HttpClient::new(&config).unwrap();

    let input_data: Vec<f32> = vec![0.0; 1 * 1 * 28 * 28];
    let infer_result: serde_json::Value = client
        .post(
            "/api/v1/inference/sync",
            &serde_json::json!({
                "model_id": model.id.to_string(),
                "inputs": {
                    "import/Placeholder:0": input_data
                }
            }),
        )
        .await
        .unwrap();

    assert!(infer_result["outputs"].is_object());
    assert!(infer_result["latency_ms"].is_number());
}
