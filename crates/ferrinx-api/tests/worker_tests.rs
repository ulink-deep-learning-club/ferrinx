//
#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::sync::Arc;

use ferrinx_common::{TaskStatus, UserRole};
use futures::future::join_all;
use uuid::Uuid;

use common::mock_redis::RedisClient;
use common::{MockInferenceEngine, MockRedis, TestDb};

#[tokio::test]
async fn test_task_message_extraction() {
    let data = HashMap::from([
        ("task_id".to_string(), Uuid::new_v4().to_string()),
        ("model_id".to_string(), Uuid::new_v4().to_string()),
    ]);

    let task_id_str = data.get("task_id").unwrap();
    let task_id = Uuid::parse_str(task_id_str);
    assert!(task_id.is_ok());
}

#[tokio::test]
async fn test_redis_add_task() {
    let redis = MockRedis::new();
    let task_id = Uuid::new_v4().to_string();

    redis.add_task("ferrinx:tasks:normal", &task_id).await;

    let count = redis.get_stream_count("ferrinx:tasks:normal").await;
    assert_eq!(count, 1);
}

#[tokio::test]
async fn test_redis_read_task() {
    let redis = MockRedis::new();
    let task_id = Uuid::new_v4().to_string();

    redis.add_task("ferrinx:tasks:normal", &task_id).await;

    let result = redis
        .xread_group("test-group", "test-consumer", "ferrinx:tasks:normal", 1, 0)
        .await
        .unwrap();

    assert!(result.is_some());
    let entries = result.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].data.get("task_id").unwrap(), &task_id);
}

#[tokio::test]
async fn test_redis_ack_task() {
    let redis = MockRedis::new();
    let task_id = Uuid::new_v4().to_string();

    redis.add_task("ferrinx:tasks:normal", &task_id).await;

    redis
        .xread_group("test-group", "test-consumer", "ferrinx:tasks:normal", 1, 0)
        .await
        .unwrap();

    redis
        .xack("ferrinx:tasks:normal", "test-group", "test-entry-id")
        .await
        .unwrap();

    let acked = redis.get_acked().await;
    assert_eq!(acked.len(), 1);
}

#[tokio::test]
async fn test_redis_result_cache() {
    let redis = MockRedis::new();
    let task_id = Uuid::new_v4().to_string();
    let result = serde_json::json!({"output": [1.0, 2.0, 3.0]});

    redis.set_result(&task_id, result.clone()).await;

    let cached = redis.get_result(&task_id).await;
    assert!(cached.is_some());
    assert_eq!(cached.unwrap(), result);
}

#[tokio::test]
async fn test_redis_health_check() {
    let redis = MockRedis::new();
    let result = redis.health_check().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_redis_dead_letter_queue() {
    let redis = MockRedis::new();
    let task_id = Uuid::new_v4().to_string();

    let mut data = HashMap::new();
    data.insert("task_id".to_string(), task_id.clone());
    data.insert("error".to_string(), "Test error".to_string());
    data.insert("retries".to_string(), "3".to_string());

    redis
        .xadd("ferrinx:tasks:dead_letter", &data)
        .await
        .unwrap();

    let count = redis.get_stream_count("ferrinx:tasks:dead_letter").await;
    assert_eq!(count, 1);
}

#[tokio::test]
async fn test_engine_successful_inference() {
    let engine = MockInferenceEngine::new(5);

    let input = ferrinx_common::InferenceInput {
        inputs: HashMap::from([("input".to_string(), serde_json::json!([1.0, 2.0, 3.0]))]),
    };

    let output = engine
        .infer("test-model", "/path/to/model.onnx", input)
        .await;

    assert!(output.is_ok());
    let output = output.unwrap();
    assert!(output.latency_ms > 0);
}

#[tokio::test]
async fn test_engine_failed_inference() {
    let engine = MockInferenceEngine::new(5);

    engine.set_should_fail(true).await;

    let input = ferrinx_common::InferenceInput {
        inputs: HashMap::from([("input".to_string(), serde_json::json!([1.0, 2.0, 3.0]))]),
    };

    let output = engine
        .infer("test-model", "/path/to/model.onnx", input)
        .await;

    assert!(output.is_err());
}

#[tokio::test]
async fn test_engine_custom_response() {
    let engine = MockInferenceEngine::new(5);

    let custom_output = ferrinx_common::InferenceOutput {
        outputs: HashMap::from([("custom_output".to_string(), serde_json::json!([42.0, 43.0]))]),
        latency_ms: 100,
    };
    engine
        .set_response("custom-model", custom_output.clone())
        .await;

    let input = ferrinx_common::InferenceInput {
        inputs: HashMap::from([("input".to_string(), serde_json::json!([1.0]))]),
    };

    let output = engine
        .infer("custom-model", "/path/to/model.onnx", input)
        .await
        .unwrap();

    assert_eq!(output.latency_ms, 100);
}

#[tokio::test]
async fn test_engine_concurrency() {
    let engine = Arc::new(MockInferenceEngine::new(2));

    engine.set_delay_ms(50).await;

    let mut handles = vec![];

    for i in 0..3 {
        let engine = engine.clone();
        let handle = tokio::spawn(async move {
            let input = ferrinx_common::InferenceInput {
                inputs: HashMap::from([("input".to_string(), serde_json::json!([i as f64]))]),
            };
            engine
                .infer(&format!("model-{}", i), "/path/to/model.onnx", input)
                .await
        });
        handles.push(handle);
    }

    let results: Vec<_> = join_all(handles).await;

    for result in results {
        assert!(result.unwrap().is_ok());
    }
}

#[tokio::test]
async fn test_task_status_transitions() {
    let test_db = TestDb::new().await;
    let user = test_db.create_user("statususer", UserRole::User).await;
    let (key_id, _) = test_db.create_api_key(&user, "status-key", false).await;
    let model = test_db.create_model("status-model", "1.0").await;
    let task = test_db.create_task(&model, &user, &key_id).await;

    assert_eq!(task.status, TaskStatus::Pending);

    test_db
        .db
        .tasks
        .update_status(&task.id, TaskStatus::Running)
        .await
        .unwrap();

    let updated = test_db
        .db
        .tasks
        .find_by_id(&task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, TaskStatus::Running);

    let outputs = serde_json::json!({"output": [1.0]});
    test_db
        .db
        .tasks
        .set_result(&task.id, TaskStatus::Completed, Some(&outputs), None)
        .await
        .unwrap();

    let completed = test_db
        .db
        .tasks
        .find_by_id(&task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.status, TaskStatus::Completed);
    assert!(completed.outputs.is_some());
    assert!(completed.completed_at.is_some());
}

#[tokio::test]
async fn test_task_failure_recording() {
    let test_db = TestDb::new().await;
    let user = test_db.create_user("failuser", UserRole::User).await;
    let (key_id, _) = test_db.create_api_key(&user, "fail-key", false).await;
    let model = test_db.create_model("fail-model", "1.0").await;
    let task = test_db.create_task(&model, &user, &key_id).await;

    test_db
        .db
        .tasks
        .set_result(&task.id, TaskStatus::Failed, None, Some("Test error"))
        .await
        .unwrap();

    let failed = test_db
        .db
        .tasks
        .find_by_id(&task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, TaskStatus::Failed);
    assert_eq!(failed.error_message, Some("Test error".to_string()));
}

#[tokio::test]
async fn test_task_count_by_status() {
    let test_db = TestDb::new().await;
    let user = test_db.create_user("countuser", UserRole::User).await;
    let (key_id, _) = test_db.create_api_key(&user, "count-key", false).await;
    let model = test_db.create_model("count-model", "1.0").await;

    for _ in 0..3 {
        test_db.create_task(&model, &user, &key_id).await;
    }

    let count = test_db
        .db
        .tasks
        .count_by_status(TaskStatus::Pending)
        .await
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_task_cleanup() {
    let test_db = TestDb::new().await;
    let user = test_db.create_user("cleanupuser", UserRole::User).await;
    let (key_id, _) = test_db.create_api_key(&user, "cleanup-key", false).await;
    let model = test_db.create_model("cleanup-model", "1.0").await;

    let task = test_db.create_task(&model, &user, &key_id).await;

    test_db
        .db
        .tasks
        .set_result(&task.id, TaskStatus::Completed, None, None)
        .await
        .unwrap();

    let cleaned = test_db.db.tasks.cleanup_expired(0, 100).await.unwrap();
    assert!(cleaned > 0);

    let count = test_db
        .db
        .tasks
        .count_by_status(TaskStatus::Completed)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_task_delete_by_user() {
    let test_db = TestDb::new().await;
    let user = test_db.create_user("deleteuser", UserRole::User).await;
    let (key_id, _) = test_db.create_api_key(&user, "delete-key", false).await;
    let model = test_db.create_model("delete-model", "1.0").await;

    test_db.create_task(&model, &user, &key_id).await;
    test_db.create_task(&model, &user, &key_id).await;

    let deleted = test_db.db.tasks.delete_by_user(&user.id).await.unwrap();
    assert_eq!(deleted, 2);
}

#[tokio::test]
async fn test_multiple_priority_streams() {
    let redis = MockRedis::new();

    let high_task = Uuid::new_v4().to_string();
    let normal_task = Uuid::new_v4().to_string();
    let low_task = Uuid::new_v4().to_string();

    redis.add_task("ferrinx:tasks:high", &high_task).await;
    redis.add_task("ferrinx:tasks:normal", &normal_task).await;
    redis.add_task("ferrinx:tasks:low", &low_task).await;

    let high_result = redis
        .xread_group("group", "consumer", "ferrinx:tasks:high", 1, 0)
        .await
        .unwrap();
    assert!(high_result.is_some());

    let normal_result = redis
        .xread_group("group", "consumer", "ferrinx:tasks:normal", 1, 0)
        .await
        .unwrap();
    assert!(normal_result.is_some());

    let low_result = redis
        .xread_group("group", "consumer", "ferrinx:tasks:low", 1, 0)
        .await
        .unwrap();
    assert!(low_result.is_some());
}

#[tokio::test]
async fn test_redis_clear() {
    let redis = MockRedis::new();

    redis
        .add_task("ferrinx:tasks:normal", &Uuid::new_v4().to_string())
        .await;
    redis
        .set_result(&Uuid::new_v4().to_string(), serde_json::json!({}))
        .await;

    redis.clear().await;

    assert_eq!(redis.get_stream_count("ferrinx:tasks:normal").await, 0);
}
