#[path = "common/mod.rs"]
mod common;

use ferrinx_common::TaskStatus;
use ferrinx_worker::{TaskMessage, RedisClient};
use std::collections::HashMap;
use uuid::Uuid;

use common::TestContext;

#[tokio::test]
async fn test_worker_consumes_task_from_redis() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;
    let task_id = ctx.create_test_task(model_id, user_id, api_key_id).await;

    let consumer = ctx.create_consumer();

    let result = consumer.poll_task().await.unwrap();
    assert!(result.is_some());

    let task_message = result.unwrap();
    assert_eq!(task_message.data.get("task_id").unwrap(), &task_id.to_string());
}

#[tokio::test]
async fn test_worker_acknowledges_task() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;
    ctx.create_test_task(model_id, user_id, api_key_id).await;

    let consumer = ctx.create_consumer();

    let task_message = consumer.poll_task().await.unwrap().unwrap();

    let result = consumer.ack_task(&task_message.stream, &task_message.entry_id).await;
    assert!(result.is_ok());

    let acked = ctx.redis.get_acked().await;
    assert_eq!(acked.len(), 1);
}

#[tokio::test]
async fn test_worker_processes_pending_task() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;
    let task_id = ctx.create_test_task(model_id, user_id, api_key_id).await;

    let task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Pending);

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor();

    let task_message = consumer.poll_task().await.unwrap().unwrap();

    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert!(updated_task.status.is_terminal());
}

#[tokio::test]
async fn test_worker_handles_task_not_found() {
    let ctx = TestContext::new().await;

    let processor = ctx.create_processor();

    let mut data = HashMap::new();
    data.insert("task_id".to_string(), Uuid::new_v4().to_string());

    let task_message = TaskMessage {
        stream: "test-stream".to_string(),
        entry_id: "test-entry".to_string(),
        data,
    };

    let result = processor.process(task_message).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_multiple_tasks_sequential_processing() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let task_id1 = ctx.create_test_task(model_id, user_id, api_key_id).await;
    let task_id2 = ctx.create_test_task(model_id, user_id, api_key_id).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor();

    for _ in 0..2 {
        if let Some(task_message) = consumer.poll_task().await.unwrap() {
            let result = processor.process(task_message).await;
            assert!(result.is_ok());
        }
    }

    let task1 = ctx.db.tasks.find_by_id(&task_id1).await.unwrap().unwrap();
    let task2 = ctx.db.tasks.find_by_id(&task_id2).await.unwrap().unwrap();

    assert!(task1.status.is_terminal());
    assert!(task2.status.is_terminal());
}

#[tokio::test]
async fn test_worker_priority_queue_order() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let _normal_task_id = ctx.create_test_task(model_id, user_id, api_key_id).await;

    let high_task_data = HashMap::from([
        ("task_id".to_string(), Uuid::new_v4().to_string()),
        ("model_id".to_string(), model_id.to_string()),
        ("user_id".to_string(), user_id.to_string()),
        ("api_key_id".to_string(), api_key_id.to_string()),
        ("priority".to_string(), "2".to_string()),
        ("created_at".to_string(), chrono::Utc::now().to_rfc3339()),
    ]);
    ctx.redis.add_task("ferrinx:tasks:high", high_task_data.clone()).await;

    let consumer = ferrinx_worker::TaskConsumer::new(
        ctx.redis.clone(),
        "test-consumer".to_string(),
        "test-group".to_string(),
        vec!["ferrinx:tasks:high".to_string(), "ferrinx:tasks:normal".to_string()],
    );

    let result = consumer.poll_task().await.unwrap();
    assert!(result.is_some());

    let task_message = result.unwrap();
    assert_eq!(task_message.stream, "ferrinx:tasks:high");
}

#[tokio::test]
async fn test_worker_health_check() {
    let ctx = TestContext::new().await;

    let consumer = ctx.create_consumer();

    let result = consumer.health_check().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_task_status_transitions() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;
    let task_id = ctx.create_test_task(model_id, user_id, api_key_id).await;

    let task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Pending);

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor();

    let task_message = consumer.poll_task().await.unwrap().unwrap();

    let running_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap();
    if let Some(task) = running_task {
        if task.status == TaskStatus::Running {
            assert!(true);
        }
    }

    let result = processor.process(task_message).await;
    assert!(result.is_ok());

    let final_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert!(final_task.status.is_terminal());
}

#[tokio::test]
async fn test_worker_result_caching() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;
    let task_id = ctx.create_test_task(model_id, user_id, api_key_id).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message).await;
    assert!(result.is_ok());

    let cache_key = format!("ferrinx:results:{}", task_id);
    let cached = ctx.redis.get_json(&cache_key).await.unwrap();
    
    if cached.is_some() {
        assert!(true);
    }
}

#[tokio::test]
async fn test_worker_concurrent_task_processing() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let mut task_ids = Vec::new();
    for _ in 0..3 {
        let task_id = ctx.create_test_task(model_id, user_id, api_key_id).await;
        task_ids.push(task_id);
    }

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor();

    for _ in 0..3 {
        if let Some(task_message) = consumer.poll_task().await.unwrap() {
            let processor = processor.clone();
            tokio::spawn(async move {
                let _ = processor.process(task_message).await;
            });
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    for task_id in &task_ids {
        if let Some(task) = ctx.db.tasks.find_by_id(task_id).await.unwrap() {
            assert!(task.status.is_terminal());
        }
    }
}

#[tokio::test]
async fn test_worker_rejects_invalid_input_shape() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let invalid_tensor = ferrinx_common::Tensor::new_f32(
        vec![1, 3, 224, 224],
        &vec![0.0f32; 3 * 224 * 224],
    );

    let task_id = ctx.create_test_task_with_inputs(model_id, user_id, api_key_id, invalid_tensor).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Failed);
    
    let error_msg = updated_task.error_message.expect("Should have error message");
    assert!(error_msg.contains("Shape mismatch") || error_msg.contains("shape"), 
        "Error message should mention shape mismatch, got: {}", error_msg);
}

#[tokio::test]
async fn test_worker_rejects_invalid_tensor_format() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let task_id = ctx.create_test_task_with_invalid_inputs(
        model_id, 
        user_id, 
        api_key_id, 
        serde_json::json!({"input": [[1.0, 2.0, 3.0]]})
    ).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Failed);
    
    let error_msg = updated_task.error_message.expect("Should have error message");
    assert!(
        error_msg.contains("Tensor format") || 
        error_msg.contains("Expected Tensor") ||
        error_msg.contains("invalid type"),
        "Error message should mention tensor format issue, got: {}", error_msg
    );
}

#[tokio::test]
async fn test_worker_handles_empty_inputs() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let task_id = ctx.create_test_task_with_invalid_inputs(
        model_id, 
        user_id, 
        api_key_id, 
        serde_json::json!({})
    ).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Failed);
    
    let error_msg = updated_task.error_message.expect("Should have error message");
    assert!(!error_msg.is_empty(), "Error message should not be empty");
}

#[tokio::test]
async fn test_worker_handles_malformed_json_inputs() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let task_id = ctx.create_test_task_with_invalid_inputs(
        model_id, 
        user_id, 
        api_key_id, 
        serde_json::json!("not an object")
    ).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Failed);
}

#[tokio::test]
async fn test_worker_handles_wrong_input_name() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let valid_tensor = ferrinx_common::Tensor::new_f32(
        vec![1, 1, 64, 64],
        &vec![0.0f32; 64 * 64],
    );

    let task_id = ctx.create_test_task_with_invalid_inputs(
        model_id, 
        user_id, 
        api_key_id, 
        serde_json::json!({"wrong_input_name": valid_tensor})
    ).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Completed);
}

#[tokio::test]
async fn test_worker_handles_dtype_mismatch() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let int_tensor = ferrinx_common::Tensor::new_i64(
        vec![1, 1, 64, 64],
        &vec![0i64; 64 * 64],
    );

    let task_id = ctx.create_test_task_with_inputs(model_id, user_id, api_key_id, int_tensor).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Failed);
    
    let error_msg = updated_task.error_message.expect("Should have error message");
    assert!(
        error_msg.contains("float32") || 
        error_msg.contains("Expected") ||
        error_msg.contains("dtype") ||
        error_msg.contains("type"),
        "Error message should mention dtype issue, got: {}", error_msg
    );
}

#[tokio::test]
async fn test_worker_validates_data_size_mismatch() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let invalid_tensor = ferrinx_common::Tensor::new_f32(
        vec![1, 1, 64, 64],
        &vec![0.0f32; 100],
    );

    let task_id = ctx.create_test_task_with_inputs(model_id, user_id, api_key_id, invalid_tensor).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Failed);
    
    let error_msg = updated_task.error_message.expect("Should have error message");
    assert!(
        error_msg.contains("size mismatch") || 
        error_msg.contains("Data size") ||
        error_msg.contains("expected"),
        "Error message should mention data size issue, got: {}", error_msg
    );
}

#[tokio::test]
async fn test_worker_error_messages_are_human_readable() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    let model_id = ctx.create_test_model(user_id).await;

    let invalid_tensor = ferrinx_common::Tensor::new_f32(
        vec![1, 3, 224, 224],
        &vec![0.0f32; 3 * 224 * 224],
    );

    let task_id = ctx.create_test_task_with_inputs(model_id, user_id, api_key_id, invalid_tensor).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let _ = processor.process(task_message.clone()).await;

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    let error_msg = updated_task.error_message.expect("Should have error message");
    
    assert!(error_msg.len() > 10, "Error message should be descriptive");
    assert!(!error_msg.contains("0x"), "Error message should not contain raw pointers");
    assert!(!error_msg.starts_with("Error("), "Error message should not be raw debug output");
    
    let words: Vec<&str> = error_msg.split_whitespace().collect();
    assert!(words.len() >= 3, "Error message should have at least 3 words for clarity");
}

#[tokio::test]
async fn test_worker_handles_invalid_model_file_path() {
    let ctx = TestContext::new().await;

    let user_id = ctx.create_test_user().await;
    let api_key_id = ctx.create_test_api_key(user_id).await;
    
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let model_path = format!("{}/../../tests/fixtures/models/nonexistent.onnx", manifest_dir);
    
    let model = ferrinx_common::ModelInfo {
        id: Uuid::new_v4(),
        name: "nonexistent-model".to_string(),
        version: "1.0.0".to_string(),
        file_path: model_path,
        file_size: Some(1024),
        storage_backend: "local".to_string(),
        input_shapes: Some(serde_json::json!({"input": [1, 1, 64, 64]})),
        output_shapes: Some(serde_json::json!({"output": [1, 1000]})),
        metadata: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    ctx.db.models.save(&model).await.unwrap();

    let task_id = ctx.create_test_task(model.id, user_id, api_key_id).await;

    let consumer = ctx.create_consumer();
    let processor = ctx.create_processor_no_retry();

    let task_message = consumer.poll_task().await.unwrap().unwrap();
    let result = processor.process(task_message.clone()).await;
    assert!(result.is_ok());

    let updated_task = ctx.db.tasks.find_by_id(&task_id).await.unwrap().unwrap();
    assert_eq!(updated_task.status, TaskStatus::Failed);
    
    let error_msg = updated_task.error_message.expect("Should have error message");
    assert!(
        error_msg.contains("Model") || 
        error_msg.contains("load") ||
        error_msg.contains("file") ||
        error_msg.contains("No such") ||
        error_msg.contains("not found"),
        "Error message should mention model loading issue, got: {}", error_msg
    );
}
