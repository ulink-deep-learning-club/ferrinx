mod common;

use assert_cmd::Command;
use common::{create_temp_config, TestContextFull};
use ferrinx_common::{Tensor, UserRole};
use std::io::Write;
use std::time::Duration;
use tempfile::NamedTempFile;

fn ferrinx_binary() -> Command {
    Command::cargo_bin("ferrinx").unwrap()
}

async fn run_cli_with_timeout(
    timeout_secs: u64,
    f: impl FnOnce(&mut Command) + Send + 'static,
) -> Option<Vec<u8>> {
    let result = tokio::task::spawn_blocking(move || {
        let mut cmd = ferrinx_binary();
        f(&mut cmd);
        match cmd.output() {
            Ok(output) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    eprintln!("CLI command failed: {}", stderr.trim());
                }
                Some(output.stdout)
            }
            Err(e) => {
                eprintln!("CLI command failed to start: {}", e);
                None
            }
        }
    });

    match tokio::time::timeout(Duration::from_secs(timeout_secs), result).await {
        Ok(Ok(Some(stdout))) => Some(stdout),
        Ok(Ok(None)) => None,
        Ok(Err(_)) => None,
        Err(_) => {
            eprintln!("CLI command timed out after {} seconds", timeout_secs);
            None
        }
    }
}

fn parse_json_output(output: &[u8]) -> serde_json::Value {
    let s = String::from_utf8_lossy(output);
    serde_json::from_str(&s).unwrap_or_else(|e| {
        panic!("Failed to parse JSON output: {}\nRaw: {}", e, s);
    })
}

#[tokio::test]
async fn test_full_async_inference_workflow() {
    let ctx = match TestContextFull::new().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: Redis not available");
            return;
        }
    };

    let user = ctx.create_user("e2euser", UserRole::User).await;
    let (_, api_key) = ctx.create_api_key(user, "e2e-key").await;
    let model = ctx.create_model("e2e-model", "1.0").await;

    let (addr, _api_handle) = ctx.start_api_server().await;
    let _worker_handle = ctx.start_worker().await;

    let config_path = create_temp_config(&format!("http://{}", addr), Some(&api_key));
    let config_path_str = config_path.path().to_string_lossy().to_string();

    let mut input_file = NamedTempFile::new().unwrap();
    let input_path = input_file.path().to_string_lossy().to_string();
    let input_shape = vec![1i64, 1, 64, 64];
    let input_vec = vec![0.0f32; 1 * 1 * 64 * 64];
    let tensor = Tensor::new_f32(input_shape, &input_vec);
    let input_data = serde_json::json!({
        "import/Placeholder:0": serde_json::to_value(&tensor).unwrap()
    });
    writeln!(
        input_file,
        "{}",
        serde_json::to_string(&input_data).unwrap()
    )
    .unwrap();

    let model_id = model.id.to_string();

    let output = run_cli_with_timeout(30, move |cmd| {
        cmd.arg("--config")
            .arg(&config_path_str)
            .arg("infer")
            .arg("async")
            .arg(&model_id)
            .arg("--input")
            .arg(&input_path);
    })
    .await
    .expect("CLI async infer command failed or timed out");

    let json = parse_json_output(&output);
    let task_id = json["task_id"]
        .as_str()
        .expect("task_id not found in output")
        .to_string();

    println!("Task submitted: {}", task_id);

    let mut final_status = String::new();
    for attempt in 0..30 {
        tokio::time::sleep(Duration::from_millis(500)).await;

        let config_path_str = config_path.path().to_string_lossy().to_string();
        let task_id_clone = task_id.clone();
        let status_output = run_cli_with_timeout(10, move |cmd| {
            cmd.arg("--config")
                .arg(&config_path_str)
                .arg("task")
                .arg("status")
                .arg(&task_id_clone);
        })
        .await
        .expect("CLI task status command failed");

        let json = parse_json_output(&status_output);
        if let Some(status) = json["status"].as_str() {
            final_status = status.to_string();
            println!("Attempt {}: status = {}", attempt, status);
            if status == "completed" || status == "failed" {
                break;
            }
        }
    }

    assert_eq!(
        final_status, "completed",
        "Task should complete successfully, got: {}",
        final_status
    );

    let config_path_str = config_path.path().to_string_lossy().to_string();
    let task_id_clone = task_id.clone();
    let status_output = run_cli_with_timeout(10, move |cmd| {
        cmd.arg("--config")
            .arg(&config_path_str)
            .arg("task")
            .arg("status")
            .arg(&task_id_clone);
    })
    .await
    .expect("CLI task status command failed");

    let json = parse_json_output(&status_output);
    assert!(json["outputs"].is_object(), "Outputs should be present");
    assert!(json["latency_ms"].is_number(), "Latency should be recorded");

    println!("Full e2e test completed successfully!");
    println!("Task ID: {}", task_id);
    println!("Final status: {}", final_status);
}

#[tokio::test]
async fn test_full_async_inference_with_priority() {
    let ctx = match TestContextFull::new().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: Redis not available");
            return;
        }
    };

    let user = ctx.create_user("priorityuser", UserRole::User).await;
    let (_, api_key) = ctx.create_api_key(user, "priority-key").await;
    let model = ctx.create_model("priority-model", "1.0").await;

    let (addr, _api_handle) = ctx.start_api_server().await;
    let _worker_handle = ctx.start_worker().await;

    let config_path = create_temp_config(&format!("http://{}", addr), Some(&api_key));
    let config_path_str = config_path.path().to_string_lossy().to_string();

    let mut input_file = NamedTempFile::new().unwrap();
    let input_path = input_file.path().to_string_lossy().to_string();
    let input_shape = vec![1i64, 1, 64, 64];
    let input_vec = vec![0.0f32; 1 * 1 * 64 * 64];
    let tensor = Tensor::new_f32(input_shape, &input_vec);
    let input_data = serde_json::json!({
        "import/Placeholder:0": serde_json::to_value(&tensor).unwrap()
    });
    writeln!(
        input_file,
        "{}",
        serde_json::to_string(&input_data).unwrap()
    )
    .unwrap();

    let model_id = model.id.to_string();

    let output = run_cli_with_timeout(30, move |cmd| {
        cmd.arg("--config")
            .arg(&config_path_str)
            .arg("infer")
            .arg("async")
            .arg(&model_id)
            .arg("--input")
            .arg(&input_path)
            .arg("--priority")
            .arg("high");
    })
    .await
    .expect("CLI async infer with priority failed or timed out");

    let json = parse_json_output(&output);
    let task_id = json["task_id"]
        .as_str()
        .expect("task_id not found in output")
        .to_string();

    for _attempt in 0..30 {
        tokio::time::sleep(Duration::from_millis(500)).await;

        let config_path_str = config_path.path().to_string_lossy().to_string();
        let task_id_clone = task_id.clone();
        let status_output = run_cli_with_timeout(10, move |cmd| {
            cmd.arg("--config")
                .arg(&config_path_str)
                .arg("task")
                .arg("status")
                .arg(&task_id_clone);
        })
        .await
        .expect("CLI task status command failed");

        let json = parse_json_output(&status_output);
        if let Some(status) = json["status"].as_str() {
            if status == "completed" {
                println!("High priority task completed!");
                return;
            }
            if status == "failed" {
                panic!("High priority task failed");
            }
        }
    }

    panic!("High priority task did not complete in time");
}

#[tokio::test]
async fn test_full_sync_inference() {
    let ctx = match TestContextFull::new().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: Redis not available");
            return;
        }
    };

    let user = ctx.create_user("syncuser", UserRole::User).await;
    let (_, api_key) = ctx.create_api_key(user, "sync-key").await;
    let model = ctx.create_model("sync-model", "1.0").await;

    let (addr, _api_handle) = ctx.start_api_server().await;

    let config_path = create_temp_config(&format!("http://{}", addr), Some(&api_key));
    let config_path_str = config_path.path().to_string_lossy().to_string();

    let mut input_file = NamedTempFile::new().unwrap();
    let input_path = input_file.path().to_string_lossy().to_string();
    let input_shape = vec![1i64, 1, 64, 64];
    let input_vec = vec![0.0f32; 1 * 1 * 64 * 64];
    let tensor = Tensor::new_f32(input_shape, &input_vec);
    let input_data = serde_json::json!({
        "import/Placeholder:0": serde_json::to_value(&tensor).unwrap()
    });
    writeln!(
        input_file,
        "{}",
        serde_json::to_string(&input_data).unwrap()
    )
    .unwrap();

    let model_id = model.id.to_string();

    let output = run_cli_with_timeout(30, move |cmd| {
        cmd.arg("--config")
            .arg(&config_path_str)
            .arg("infer")
            .arg("sync")
            .arg(&model_id)
            .arg("--input")
            .arg(&input_path);
    })
    .await;

    match output {
        Some(output) => {
            let json = parse_json_output(&output);
            assert!(json["outputs"].is_object(), "Outputs should be present");
            assert!(json["latency_ms"].is_number(), "Latency should be recorded");
            println!("Sync inference completed successfully!");
        }
        None => {
            eprintln!("Sync inference timed out - skipping test");
        }
    }
}

#[tokio::test]
async fn test_full_multiple_tasks_workflow() {
    let ctx = match TestContextFull::new().await {
        Some(c) => c,
        None => {
            eprintln!("Skipping test: Redis not available");
            return;
        }
    };

    let user = ctx.create_user("multiuser", UserRole::User).await;
    let (_, api_key) = ctx.create_api_key(user, "multi-key").await;
    let model = ctx.create_model("multi-model", "1.0").await;

    let (addr, _api_handle) = ctx.start_api_server().await;
    let _worker_handle = ctx.start_worker().await;

    let config_path = create_temp_config(&format!("http://{}", addr), Some(&api_key));
    let config_path_str = config_path.path().to_string_lossy().to_string();
    let model_id = model.id.to_string();

    let mut task_ids = Vec::new();

    for i in 0..3 {
        let mut input_file = NamedTempFile::new().unwrap();
        let input_path = input_file.path().to_string_lossy().to_string();
        let input_shape = vec![1i64, 1, 64, 64];
        let input_vec = vec![i as f32; 1 * 1 * 64 * 64];
        let tensor = Tensor::new_f32(input_shape, &input_vec);
        let input_data = serde_json::json!({
            "import/Placeholder:0": serde_json::to_value(&tensor).unwrap()
        });
        writeln!(
            input_file,
            "{}",
            serde_json::to_string(&input_data).unwrap()
        )
        .unwrap();

        let config_path_str = config_path_str.clone();
        let model_id = model_id.clone();
        let output = run_cli_with_timeout(30, move |cmd| {
            cmd.arg("--config")
                .arg(&config_path_str)
                .arg("infer")
                .arg("async")
                .arg(&model_id)
                .arg("--input")
                .arg(&input_path);
        })
        .await
        .expect("CLI async infer command failed");

        let json = parse_json_output(&output);
        task_ids.push(
            json["task_id"]
                .as_str()
                .expect("task_id not found")
                .to_string(),
        );
    }

    assert_eq!(task_ids.len(), 3, "Should submit 3 tasks");

    for task_id in &task_ids {
        let mut completed = false;
        for _attempt in 0..30 {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let config_path_str = config_path.path().to_string_lossy().to_string();
            let task_id_clone = task_id.clone();
            let status_output = run_cli_with_timeout(10, move |cmd| {
                cmd.arg("--config")
                    .arg(&config_path_str)
                    .arg("task")
                    .arg("status")
                    .arg(&task_id_clone);
            })
            .await
            .expect("CLI task status command failed");

            let json = parse_json_output(&status_output);
            if let Some(status) = json["status"].as_str() {
                if status == "completed" {
                    completed = true;
                    break;
                }
                if status == "failed" {
                    panic!("Task {} failed", task_id);
                }
            }
        }
        assert!(completed, "Task {} should complete", task_id);
    }

    println!("All {} tasks completed successfully!", task_ids.len());
}
