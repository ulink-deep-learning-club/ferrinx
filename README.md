# Ferrinx

A high-performance ONNX inference backend service built in Rust, featuring both synchronous and asynchronous inference modes, RESTful API, and CLI client.

## Features

- **Dual Inference Modes**: 
  - Sync: Low-latency (<100ms), runs in API process, local models only
  - Async: Distributed, routes to best available Worker, supports model routing
- **Intelligent Model Routing**: Async tasks are routed to Workers based on model availability:
  - Priority: cached → available → error
- **Graceful Degradation**: 
  - Without Redis: operates as simple inference engine (sync only)
  - With Redis: full distributed async inference
- **High Performance**: Built on `ort` (ONNX Runtime bindings for Rust) with `spawn_blocking` for CPU-intensive inference
- **Scalable Architecture**: Independent worker processes with model-aware task distribution
- **Flexible Storage**: Local filesystem storage for model files (pre-distributed models)
- **Database Agnostic**: PostgreSQL for production, SQLite for development/testing
- **API Key Authentication**: Secure authentication with Redis caching and database fallback
- **CLI Client**: Lightweight command-line tool for administration and inference

## Architecture

Ferrinx supports two deployment modes:

### Simplified Mode (No Redis)

When Redis is unavailable, Ferrinx operates as a simple inference engine with HTTP interface:

```
┌─────────────────────────────────────────────────────────────┐
│                        Client Layer                         │
│       CLI Tool                RESTful Client                │
└──────────┬───────────────────────────┬──────────────────────┘
           │ HTTP/JSON                 │ HTTP/JSON
           ▼                           ▼
┌─────────────────────────────────────────────────────────────┐
│                      API Server (axum)                      │
│  - API Key Validation (DB only)                             │
│  - Sync Inference ✅ (in-process)                           │
│  - Async Inference ❌ (unavailable)                         │
│  - InferenceEngine + Model Storage                          │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                   Infrastructure Layer                      │
│        Database (PG/SQLite)        Model Storage (Local)    │
└─────────────────────────────────────────────────────────────┘
```

### Full Mode (With Redis)

With Redis, Ferrinx provides distributed async inference with intelligent model routing:

```
┌─────────────────────────────────────────────────────────────┐
│                        Client Layer                         │
│       CLI Tool                RESTful Client                │
└──────────┬───────────────────────────┬──────────────────────┘
           │ HTTP/JSON                 │ HTTP/JSON
           ▼                           ▼
┌─────────────────────────────────────────────────────────────┐
│                      API Server (axum)                      │
│  - API Key Validation (Redis cache + DB fallback)           │
│  - Sync Inference ✅ (in-process, local models only)        │
│  - Async Inference ✅ (route to Workers via Redis)          │
│  - Model Routing: task → best available Worker              │
└──────────────────────────┬──────────────────────────────────┘
                           │
           ┌───────────────┴───────────────┐
           ▼                               ▼
┌──────────────────────┐    ┌──────────────────────────────────┐
│  Sync Inference      │    │     Async Inference Path         │
│  (Low Latency)       │    │                                  │
│  - In-process cache  │    │  Redis Streams (model-specific)  │
│  - Local models only │    │         ↓                        │
└──────────────────────┘    │  Worker Pool (model-aware)       │
                            │  - Worker A: models [X, Y]       │
                            │  - Worker B: models [Y, Z]       │
                            │  - Worker C: models [X, Z]       │
                            └──────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                   Infrastructure Layer                      │
│  Redis (Streams/Cache)  Database (PG/SQLite)  Model Storage │
└─────────────────────────────────────────────────────────────┘
```

### Model Routing Strategy (Async Inference Only)

When submitting an async inference task, the system routes it to the best available Worker:

1. **Priority 1**: Worker with model already cached in memory (fastest)
2. **Priority 2**: Worker with model file available (needs loading)
3. **No Worker available**: Returns error `NO_WORKER_AVAILABLE`

Workers report their model status to Redis:
- Which models they have access to (file exists)
- Which models are cached in memory

```
Redis Key: ferrinx:workers:{worker_id}:models
Value: {
  "model_uuid_1": "cached",    # loaded in memory
  "model_uuid_2": "available", # file exists, not loaded
  "model_uuid_3": "available"
}
```

## Project Structure

```
ferrinx/
├── Cargo.toml              # Workspace configuration
├── config.example.toml     # Example configuration
├── design.md               # Architecture design document
│
└── crates/
    ├── ferrinx-common/     # Shared types, config, utilities
    ├── ferrinx-db/         # Database abstraction layer
    ├── ferrinx-core/       # Inference engine core
    ├── ferrinx-api/        # RESTful API server
    ├── ferrinx-worker/     # Async inference worker
    └── ferrinx-cli/        # Command-line client
```

### Module Dependencies

```
ferrinx-common  ← (shared by all crates)
    ↑
ferrinx-db      ← (database abstraction)
    ↑
ferrinx-core    ← (inference engine)
    ↑
┌───┴────┐
▼        ▼
ferrinx-api     ferrinx-worker
│
▼
ferrinx-cli     ← (HTTP client only, no core/db dependency)
```

## Quick Start

### Prerequisites

- Rust 1.70+
- PostgreSQL 14+ (or SQLite 3.35+ for development)
- Redis 6.2+
- ONNX Runtime

### Installation

```bash
# Clone the repository
git clone https://github.com/your-org/ferrinx.git
cd ferrinx

# Build
cargo build --release

# Run database migrations
./target/release/ferrinx db migrate

# Start API server
./target/release/ferrinx-api

# Start worker (in another terminal)
./target/release/ferrinx-worker
```

### Bootstrap

```bash
# Create first admin user
curl -X POST http://localhost:8080/api/v1/bootstrap \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "your-password"}'

# Save the returned API key
export FERRINX_API_KEY="frx_sk_..."
```

### CLI Usage

```bash
# Configure CLI
ferrinx config set api-url http://localhost:8080/api/v1
ferrinx config set api-key $FERRINX_API_KEY

# Upload a model
ferrinx model upload ./model.onnx --name my-model --version 1.0

# Synchronous inference
ferrinx infer my-model:1.0 --input '{"input.1": [[1.0, 2.0, 3.0]]}'

# Asynchronous inference
ferrinx infer my-model:1.0 --input ./input.json --async
ferrinx task status <task-id>
```

## API Endpoints

### Authentication

```
POST /api/v1/bootstrap          # Create first admin (available once)
POST /api/v1/auth/login         # Login with username/password
POST /api/v1/auth/logout        # Logout and invalidate temp key
```

### Models

```
POST   /api/v1/models/upload    # Upload model file
POST   /api/v1/models/register  # Register existing model
GET    /api/v1/models           # List models
GET    /api/v1/models/{id}      # Get model details
DELETE /api/v1/models/{id}      # Delete model
```

### Inference

```
POST   /api/v1/inference/sync   # Synchronous inference
POST   /api/v1/inference        # Asynchronous inference
GET    /api/v1/inference/{id}   # Get inference result
DELETE /api/v1/inference/{id}   # Cancel task
GET    /api/v1/inference        # List tasks
```

### Admin

```
POST   /api/v1/admin/users      # Create user (admin only)
GET    /api/v1/admin/users      # List users
DELETE /api/v1/admin/users/{id} # Delete user
```

### API Keys

```
POST   /api/v1/api-keys         # Create API key
GET    /api/v1/api-keys         # List user's API keys
DELETE /api/v1/api-keys/{id}    # Revoke API key
```

## Configuration

See `config.example.toml` for full configuration options.

```toml
[server]
host = "0.0.0.0"
port = 8080
sync_inference_concurrency = 4
sync_inference_timeout = 30

[database]
backend = "postgresql"
url = "${FERRINX_DATABASE_URL}"

[redis]
url = "${FERRINX_REDIS_URL}"

[storage]
backend = "local"
path = "./models"

[onnx]
cache_size = 5
execution_provider = "CPU"
```

## Example: Synchronous Inference

```bash
curl -X POST http://localhost:8080/api/v1/inference/sync \
  -H "Authorization: Bearer frx_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
    "model_id": "model-uuid",
    "inputs": {
      "input.1": [[1.0, 2.0, 3.0]]
    }
  }'
```

Response:
```json
{
  "request_id": "req-abc-123",
  "data": {
    "outputs": {
      "output.1": [[0.5, 0.3, 0.2]]
    },
    "latency_ms": 45
  }
}
```

## Example: Asynchronous Inference

```bash
# Submit task
curl -X POST http://localhost:8080/api/v1/inference \
  -H "Authorization: Bearer frx_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
    "model_id": "model-uuid",
    "inputs": {"input.1": [[1.0, 2.0]]},
    "options": {"priority": "high"}
  }'

# Response
{"request_id": "req-xxx", "data": {"task_id": "task-456", "status": "pending"}}

# Poll for result
curl http://localhost:8080/api/v1/inference/task-456 \
  -H "Authorization: Bearer frx_sk_..."
```

## Development

```bash
# Run tests
cargo test

# Run with SQLite (development)
export FERRINX_DATABASE_URL="sqlite://./data/ferrinx.db"
cargo run --bin ferrinx-api

# Enable S3 storage
cargo build --features s3-storage
```

## Tech Stack

| Component | Technology |
|-----------|------------|
| Web Framework | axum |
| ONNX Runtime | ort |
| Database | sqlx (PostgreSQL/SQLite) |
| Cache/Queue | redis (Redis Streams) |
| Async Runtime | tokio |
| CLI | clap |
| Serialization | serde |
| Logging | tracing |

## Deployment

### Single Node

Suitable for development and small-scale production:
- API Server + Worker on same machine
- PostgreSQL single node
- Redis single node
- Local storage

### Distributed

For high-availability and scaling:
- Multiple API servers behind load balancer (consistent hashing for sync inference)
- Multiple workers consuming from Redis Streams
- Redis Cluster for HA
- NFS/shared storage for model files

## Security

- API Key authentication with SHA-256 hash storage
- Redis caching with database fallback for resilience
- Rate limiting per API key
- Request size limits
- SQL injection prevention via parameterized queries
- Password hashing with bcrypt

## License

Apache-2.0

## Incomplete Features

The following features are planned but not yet fully implemented. Contributions welcome!

### High Priority

| Feature | Location | Description |
|---------|----------|-------------|
| GPU Execution Providers | `ferrinx-core/src/inference/engine.rs` | CUDA and TensorRT execution providers are defined but not configured. |
| NMS Postprocessing | `ferrinx-core/src/transform/pipeline.rs:298` | Non-Maximum Suppression for object detection models returns unsupported error. |

### Medium Priority

| Feature | Location | Description |
|---------|----------|-------------|
| Prometheus Metrics | `ferrinx-api/src/handlers/mod.rs:36` | `/api/v1/metrics` endpoint is a stub. Full metrics defined in design docs. |
| Model Metadata Extraction | `ferrinx-core/src/model/loader.rs:93` | `opset_version` and `producer_name` always return `None`. |
| Dynamic Batching | Not implemented | `DynamicBatcher` for batching requests described in design docs. |
| `update_retry_count` | `ferrinx-db/src/traits.rs` | DB repository method mentioned in design but not in trait. |

### Low Priority

| Feature | Location | Description |
|---------|----------|-------------|
| Lua Scripting | Not implemented | Custom pre/post-processing via Lua scripts. |
| Model Optimizer | Not implemented | Quantization (INT8), graph optimization. |
| Version Aliases | Not implemented | Model version aliasing (e.g., `production` tag). |
| OpenTelemetry | Not implemented | Distributed tracing with Jaeger/Zipkin/OTLP. |
| Letterbox Preprocessing | Not implemented | Aspect-ratio preserving resize for images. |

## Documentation

- [Architecture Design](design.md) - Detailed architecture and design decisions
