// E2E tests for ferrinx CLI - testing the actual binary commands

use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use tempfile::NamedTempFile;

mod common;
use common::{lenet_model_path, models_dir, TestApp};
use ferrinx_common::UserRole;

fn ferrinx_binary() -> Command {
    Command::cargo_bin("ferrinx").unwrap()
}

fn create_temp_config(api_base: &str, api_key: Option<&str>) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    // CLI expects the full API URL including /api/v1 path
    let api_url = if api_base.ends_with("/api/v1") {
        api_base.to_string()
    } else {
        format!("{}/api/v1", api_base)
    };
    let content = if let Some(key) = api_key {
        format!(
            r#"
api_url = "{}"
api_key = "{}"
timeout = 5
output_format = "json"
"#,
            api_url, key
        )
    } else {
        format!(
            r#"
api_url = "{}"
timeout = 5
output_format = "json"
"#,
            api_url
        )
    };
    file.write_all(content.as_bytes()).unwrap();
    file
}

#[tokio::test]
async fn test_cli_bootstrap_command() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("bootstrap")
        .assert()
        .success()
        .stdout(predicate::str::contains("System initialized successfully"))
        .stdout(predicate::str::contains("API key:"));
}

#[tokio::test]
async fn test_cli_status_command() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Status:"))
        .stdout(predicate::str::contains("Version:"));
}

#[tokio::test]
async fn test_cli_auth_login_command() {
    let test_app = TestApp::new().await;
    test_app.db.create_user("testuser", UserRole::User).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("auth")
        .arg("login")
        .arg("--username")
        .arg("testuser")
        .arg("--password")
        .arg("password")
        .assert()
        .success()
        .stdout(predicate::str::contains("Login successful"));
}

#[tokio::test]
async fn test_cli_model_list_empty() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("list")
        .assert()
        .success();
}

#[tokio::test]
async fn test_cli_model_register_and_list() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));
    let model_path = lenet_model_path();

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("register")
        .arg(&model_path)
        .arg("--name")
        .arg("test-model")
        .arg("--version")
        .arg("1.0.0")
        .assert()
        .success()
        .stdout(predicate::str::contains("Model registered"));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("test-model"));
}

#[tokio::test]
async fn test_cli_model_delete() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let model = test_app.db.create_model("delete-model", "1.0", Some(test_app.storage_path.path())).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("delete")
        .arg("--name")
        .arg(&model.name)
        .arg("--version")
        .arg(&model.version)
        .assert()
        .success()
        .stdout(predicate::str::contains("Model deleted"));
}

#[tokio::test]
async fn test_cli_api_key_create_and_list() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("keyuser", UserRole::User).await;
    let (_, user_key) = test_app.db.create_api_key(&user, "user-key", false).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&user_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("api-key")
        .arg("create")
        .arg("--name")
        .arg("test-api-key")
        .assert()
        .success()
        .stdout(predicate::str::contains("API key created"));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("api-key")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("test-api-key"));
}

#[tokio::test]
async fn test_cli_inference_sync() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("inferuser", UserRole::User).await;
    let (_, user_key) = test_app.db.create_api_key(&user, "infer-key", false).await;
    let model = test_app.db.create_model("infer-model", "1.0", Some(test_app.storage_path.path())).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&user_key));

    let mut input_file = NamedTempFile::new().unwrap();
    let input_data = serde_json::json!({
        "import/Placeholder:0": vec![0.0f32; 1 * 1 * 28 * 28]
    });
    writeln!(input_file, "{}", serde_json::to_string(&input_data).unwrap()).unwrap();

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("infer")
        .arg("sync")
        .arg(&model.id.to_string())
        .arg("--input")
        .arg(input_file.path())
        .assert()
        .success();
}

#[tokio::test]
async fn test_cli_admin_create_user() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("admin")
        .arg("create-user")
        .arg("--username")
        .arg("newuser")
        .arg("--password")
        .arg("securepass")
        .arg("--role")
        .arg("user")
        .assert()
        .success()
        .stdout(predicate::str::contains("User created"));
}

#[tokio::test]
async fn test_cli_admin_list_users() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("admin")
        .arg("list-users")
        .assert()
        .success()
        .stdout(predicate::str::contains("admin"));
}

#[tokio::test]
async fn test_cli_task_list() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("taskuser", UserRole::User).await;
    let (key_id, user_key) = test_app.db.create_api_key(&user, "task-key", false).await;
    let model = test_app.db.create_model("task-model", "1.0", Some(test_app.storage_path.path())).await;
    test_app.db.create_task(&model, &user, &key_id).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&user_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("task")
        .arg("list")
        .assert()
        .success();
}

#[tokio::test]
async fn test_cli_error_no_api_key() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Error"));
}

#[tokio::test]
async fn test_cli_error_invalid_api_key() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some("invalid-key"));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Error"));
}

#[tokio::test]
async fn test_cli_output_format_json() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("--output")
        .arg("json")
        .arg("model")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("["));
}

#[tokio::test]
async fn test_cli_config_show() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("config")
        .arg("show")
        .assert()
        .success()
        .stdout(predicate::str::contains("api_url"));
}

#[tokio::test]
async fn test_cli_config_set() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("config")
        .arg("set")
        .arg("timeout")
        .arg("5")
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration updated"));
}

#[tokio::test]
async fn test_cli_global_url_override() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config("http://wrong-url:9999", None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("--url")
        .arg(format!("http://{}/api/v1", addr))
        .arg("--api-key")
        .arg(&admin_key)
        .arg("status")
        .assert()
        .success();
}

#[tokio::test]
async fn test_cli_global_api_key_override() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("--api-key")
        .arg(&admin_key)
        .arg("model")
        .arg("list")
        .assert()
        .success();
}

#[tokio::test]
async fn test_cli_version_flag() {
    ferrinx_binary()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("ferrinx"));
}

#[tokio::test]
async fn test_cli_help_flag() {
    ferrinx_binary()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Ferrinx ONNX Inference CLI"));
}

#[tokio::test]
async fn test_cli_model_info() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let model = test_app.db.create_model("info-model", "1.0", Some(test_app.storage_path.path())).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("info")
        .arg(model.id.to_string())
        .assert()
        .success()
        .stdout(predicate::str::contains("info-model"));
}

#[tokio::test]
async fn test_cli_api_key_revoke() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("revokeuser", UserRole::User).await;
    let (_, user_key) = test_app.db.create_api_key(&user, "user-key", false).await;
    let (key_to_revoke, _) = test_app.db.create_api_key(&user, "revoke-key", false).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&user_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("api-key")
        .arg("revoke")
        .arg(key_to_revoke.to_string())
        .assert()
        .success()
        .stdout(predicate::str::contains("API key revoked"));
}

#[tokio::test]
async fn test_cli_permission_denied_for_user() {
    let test_app = TestApp::new().await;
    let user = test_app.db.create_user("regularuser", UserRole::User).await;
    let (_, user_key) = test_app.db.create_api_key(&user, "user-key", false).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&user_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("admin")
        .arg("create-user")
        .arg("--username")
        .arg("unauthorized")
        .arg("--password")
        .arg("test")
        .arg("--role")
        .arg("user")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Error"));
}

#[tokio::test]
async fn test_cli_full_workflow() {
    let test_app = TestApp::new().await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), None);

    let bootstrap_output = ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("bootstrap")
        .assert()
        .success()
        .get_output()
        .stdout.clone();

    let output_str = String::from_utf8_lossy(&bootstrap_output);
    let api_key_start = output_str
        .find("API key:")
        .expect("API key not found in bootstrap output");
    let api_key_line = &output_str[api_key_start..];
    let api_key: String = api_key_line
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let config_with_key = create_temp_config(&format!("http://{}", addr), Some(&api_key));

    ferrinx_binary()
        .arg("--config")
        .arg(config_with_key.path())
        .arg("status")
        .assert()
        .success();

    ferrinx_binary()
        .arg("--config")
        .arg(config_with_key.path())
        .arg("model")
        .arg("list")
        .assert()
        .success();
}

#[tokio::test]
async fn test_cli_model_upload_with_config() {
    let test_app = TestApp::new().await;
    let admin = test_app.db.create_user("admin", UserRole::Admin).await;
    let (_, admin_key) = test_app.db.create_api_key(&admin, "admin-key", true).await;
    let (addr, _handle) = test_app.start_server_blocking();
    let config_file = create_temp_config(&format!("http://{}", addr), Some(&admin_key));
    let model_path = lenet_model_path();
    let config_path = models_dir().join("lenet.toml");

    ferrinx_binary()
        .arg("--config")
        .arg(config_file.path())
        .arg("model")
        .arg("upload")
        .arg(&model_path)
        .arg("--name")
        .arg("upload-test-model")
        .arg("--version")
        .arg("1.0.0")
        .arg("--model-config")
        .arg(config_path.to_str().unwrap())
        .assert()
        .success()
        .stdout(predicate::str::contains("Model uploaded"));
}
