# Integration Tests

This directory contains integration tests for the Ferrinx project.

## Test Structure

```
tests/integration/
├── mod.rs                    # Module entry point
├── api_tests.rs              # API integration tests
├── worker_tests.rs           # Worker integration tests  
├── cli_tests.rs              # CLI integration tests
├── e2e_tests.rs              # End-to-end tests
└── fixtures/
    ├── mod.rs                # Test fixtures module
    ├── mock_engine.rs        # Mock inference engine
    ├── mock_redis.rs         # Mock Redis client
    ├── test_app.rs           # Test application builder
    └── test_db.rs            # Test database setup
```

## Running Tests

```bash
# Run all integration tests
cargo test --test integration

# Run specific test file
cargo test --test integration::api_tests

# Run specific test
cargo test --test integration::api_tests::test_bootstrap_creates_admin
```

## Test Categories

### API Tests (`api_tests.rs`)
- Health check endpoints (`/api/v1/health`, `/api/v1/ready`)
- Bootstrap flow (first admin creation)
- Login flow
- API Key management (create, list, revoke)
- Model management (register, list, delete)
- Sync inference
- Permission control (user vs admin)

### Worker Tests (`worker_tests.rs`)
- Task message handling
- Redis task queue operations
- Inference engine mock behavior
- Task status transitions
- Task cleanup
- Multiple priority streams

### CLI Tests (`cli_tests.rs`)
- Configuration management
- HTTP client operations
- Input parsing
- Error handling
- Bootstrap flow

### E2E Tests (`e2e_tests.rs`)
- Complete bootstrap → login flow
- Full API key workflow
- Model management lifecycle
- Sync inference workflow
- Admin user management
- Task lifecycle
- Permission isolation
- Concurrent requests
- Error responses

## Test Infrastructure

### MockInferenceEngine
Provides a mock implementation of the inference engine for testing without ONNX models.

### MockRedis
Provides an in-memory Redis implementation for testing without a real Redis server.

### TestDb
Creates a temporary SQLite database with migrations for isolated testing.

### TestApp
Starts a test API server on a random port for HTTP integration tests.

## Environment Variables

Tests can be configured via environment variables:

- `TEST_API_URL`: Override default API URL
- `TEST_DB_PATH`: Use specific database path
- `TEST_REDIS_URL`: Use specific Redis URL

## Dependencies

Key test dependencies:
- `tokio` - Async runtime
- `tempfile` - Temporary files/directories
- `reqwest` - HTTP client
- `serde_json` - JSON serialization
- `futures` - Async utilities
