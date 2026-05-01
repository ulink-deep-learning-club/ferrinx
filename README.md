# Ferrinx

A lightweight toy ONNX inference service with simplicity and flexibility in mind, while maintaining decent performance.

Turn your ONNX models into HTTP APIs with minimal fuss. One binary, one config file, and you're running.

## Quick Start

```bash
# 1. Build
cargo build --release

# 2. Run API server
./target/release/ferrinx-api

# 3. Bootstrap (creates admin user, saves API key to config, initial password is printed to console)
./target/release/ferrinx bootstrap  # Using ferrinx cli

# 4. Login (if not using bootstrap)
./target/release/ferrinx auth login -U admin

# 5. Register a model (when API and CLI are on the same machine)
./target/release/ferrinx model register --model-config tests/fixtures/models/hanzi-tiny.toml

# 6. Run inference
./target/release/ferrinx infer sync --name hanzi-tiny --version 1.0 --image tests/fixtures/models/#U4e16.jpg  # An image of character 世
```

That's it. No Docker, no complex config files, no model repository layout.

## Architecture

Ferrinx keeps it simple with two deployment modes:

### Simple Mode (No Redis)

Just run the binary and go. Perfect for development, testing, or when you don't want extra infrastructure:

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

### Distributed Mode (With Redis)

When you need to scale beyond a single machine, add Redis for task distribution:

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

### How Model Routing Works

In distributed mode, tasks are sent to the best available worker:

1. **Fastest**: Workers with the model already loaded in memory
2. **Next best**: Workers that have the model file (will load it)
3. **No luck**: Returns an error if no worker can handle the model

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
ferrinx-common  ← (shared types: ModelConfig, DTOs, config, etc.)
    ↑
├── ferrinx-db      (database abstraction)
│       ↑
│   ferrinx-core    (inference engine - ONNX Runtime)
│       ↑
│   ┌───┴────┐
│   ferrinx-api     ferrinx-worker
│
└── ferrinx-cli     (HTTP client - NO core/api dependencies)
```

**Note:** `ferrinx-cli` is fully independent of `ferrinx-core` and `ferrinx-api`:
- All shared types (`ModelConfig`, `ApiKeyDetail`, `TaskDetail`, etc.) are in `ferrinx-common`
- CLI only depends on `ferrinx-common` for production builds
- CLI test suite uses `ferrinx-core`/`ferrinx-api`/`ferrinx-db` for integration tests

## Quick Start

### Prerequisites

- Rust 1.70+
- SQLite 3.35+ (default, for development)
- Redis 6.2+ (optional, for distributed mode)
- ONNX Runtime

### ONNX Runtime Linking

Ferrinx supports two linking modes for ONNX Runtime:

#### Static Linking (Default)

By default, Ferrinx uses the `download-binaries` feature which statically links pre-built ONNX Runtime binaries. This is the simplest approach but requires:

- **glibc 2.38+** (Debian 13+, Ubuntu 24.04+, Fedora 39+, etc.)

```bash
# Default build - uses pre-built ONNX Runtime binaries
cargo build --release
```

#### Dynamic Linking (load-dynamic)

For systems with older glibc versions, use the `load-dynamic` feature to load your system's ONNX Runtime at runtime:

1. Install ONNX Runtime on your system (e.g., from [official releases](https://github.com/microsoft/onnxruntime/releases))

2. Build with `load-dynamic` feature:
```bash
cargo build --release --features load-dynamic
```

3. Configure the library path (priority order):
   - **Config file** (`config.toml`) - highest priority:
     ```toml
     [onnx]
     dynamic_lib_path = "/path/to/libonnxruntime.so"
     ```
   - **Environment variable** `ORT_DYLIB_PATH`:
     ```bash
     ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so ./target/release/ferrinx-api
     ```
   - **System library path** (`LD_LIBRARY_PATH` on Linux):
     ```bash
     LD_LIBRARY_PATH=/usr/local/lib ./target/release/ferrinx-api
     ```

### Execution Providers

Ferrinx supports multiple execution providers for hardware acceleration:

| Provider | Feature Flag | Platform | Notes |
|----------|-------------|----------|-------|
| CPU | (default) | All | Always available |
| WebGPU | `--features webgpu` | Linux, Windows, macOS | Uses Vulkan (Linux), DirectX (Windows), Metal (macOS) |
| CUDA | `--features cuda` | Linux, Windows | CUDA + cuDNN required (version depends on ONNX Runtime version) |
| CoreML | `--features coreml` | macOS | Apple Silicon optimization |
| ROCm | `--features rocm` | Linux | AMD GPU support |

**CUDA/cuDNN requirements by ONNX Runtime version:**

| API Version | CUDA | cuDNN |
|-------------|------|-------|
| `api-17` - `api-21` | 11.x | 8.x |
| `api-22` - `api-23` | 12.x | 8.9+ |
| `api-24` | 12.8+ | 9.19+ |

**WebGPU** is the recommended GPU acceleration option for most users:
```bash
# Build with WebGPU support
cargo build --release --features webgpu

# Run (Linux requires LD_LIBRARY_PATH for the WebGPU library)
LD_LIBRARY_PATH=./target/release ./target/release/ferrinx-api
```

Configure in `config.toml`:
```toml
[onnx]
execution_provider = "WEBGPU"
```

### ONNX Runtime API Versions

Ferrinx supports multiple ONNX Runtime versions via API version features. Only one API version can be enabled at a time:

| Feature | ONNX Runtime Version | Notes |
|---------|---------------------|-------|
| `api-17` | 1.17 | Baseline |
| `api-18` | 1.18 | |
| `api-19` | 1.19 | |
| `api-20` | 1.20 | |
| `api-21` | 1.21 | |
| `api-22` | 1.22 | |
| `api-23` | 1.23 | **Default** - Last version supporting NVIDIA Pascal (GP100, GP102, etc.) and Volta (GV100) architectures |
| `api-24` | 1.24 | Latest |

**Build examples:**
```bash
# Default build (api-23, ONNX Runtime 1.23)
cargo build --release

# ONNX Runtime 1.20 with CUDA
cargo build --release --no-default-features --features "api-20,cuda"

# Latest ONNX Runtime 1.24 with CoreML
cargo build --release --no-default-features --features "api-24,coreml"

# ONNX Runtime 1.17 (oldest supported) with ROCm
cargo build --release --no-default-features --features "api-17,rocm"
```

> **Note:** If you have older NVIDIA GPUs (Pascal/Volta architecture), use the default `api-23` or explicitly enable it, as ONNX Runtime 1.24+ dropped support for these architectures.

### Installation

```bash
# Clone the repository
git clone https://github.com/ulink-deep-learning-club/ferrinx.git
cd ferrinx

# Build
cargo build --release

# Start API server (SQLite database is auto-created)
./target/release/ferrinx-api

# Start worker (in another terminal, requires Redis for async inference)
./target/release/ferrinx-worker
```

### Worker Configuration

Worker uses the same `ferrinx.toml` configuration file. See [docs/WORKER.md](docs/WORKER.md) for detailed worker configuration options.

Quick worker configuration:

```toml
[worker]
consumer_name = ""           # Auto-generated if empty (hostname-pid)
concurrency = 4              # Number of concurrent tasks
poll_interval_ms = 100       # Redis polling interval
max_retries = 3              # Task retry count
retry_delay_ms = 1000        # Retry delay
```

### Bootstrap

#### Using CLI (Recommended)

```bash
# Create first admin user and save API key to config
./target/release/ferrinx bootstrap
```

#### Using curl

```bash
# Create first admin user (returns secure random password)
curl -X POST http://localhost:8080/api/v1/bootstrap \
  -H "Content-Type: application/json" \
  -d '{}'

# Save the returned API key
export FERRINX_API_KEY="frx_sk_..."
```

### CLI Usage

```bash
# Configure CLI (optional if already bootstrapped)
./target/release/ferrinx config set api-url http://localhost:8080/api/v1
./target/release/ferrinx config set api-key $FERRINX_API_KEY

# Register a model (when API and CLI on same machine)
./target/release/ferrinx model register --model-config ./hanzi-tiny.toml

# Upload a model (when model is on client machine)
./target/release/ferrinx model upload ./model.onnx --name my-model --version 1.0

# Synchronous inference with image
./target/release/ferrinx infer sync --name hanzi-tiny --version 1.0 --image ./test.jpg

# Synchronous inference with JSON input
./target/release/ferrinx infer sync <model-id> --input '{"input": {...}}'

# Asynchronous inference
./target/release/ferrinx infer async --name my-model --version 1.0 --input ./input.json
./target/release/ferrinx task status <task-id>
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
execution_provider = "CPU"  # Options: CPU, CUDA, TensorRT, CoreML, ROCm
# dynamic_lib_path = "/path/to/libonnxruntime.so"  # Optional: for load-dynamic feature

[worker]
consumer_name = ""           # Worker ID (auto-generated if empty)
concurrency = 4              # Concurrent tasks
poll_interval_ms = 100       # Redis poll interval
max_retries = 3              # Task retry count
```

See [docs/WORKER.md](docs/WORKER.md) for complete worker configuration.

## Example: Synchronous Inference

### Using CLI (Recommended)

```bash
# Using model name and version
./target/release/ferrinx infer sync \
  --name hanzi-tiny --version 1.0 \
  --image ./image.jpg

# Or using model ID
./target/release/ferrinx infer sync <model-uuid> \
  --input '{"input": {"dtype": "float32", "shape": [1, 1, 64, 64], "data": "base64..."}}'

# Save output to file
./target/release/ferrinx infer sync \
  --name hanzi-tiny --version 1.0 \
  --image ./image.jpg --output result.json
```

Response:
```json
{
  "result": {
    "class_index": 11,
    "label": "#U4e16",
    "probability": 0.9898158311843872
  },
  "latency_ms": 10
}
```

### Using curl (Raw API)

> **Note**: Tensor data must be base64-encoded in the raw API. Use CLI for convenience.

```bash
curl -X POST http://localhost:8080/api/v1/inference/sync \
  -H "Authorization: Bearer frx_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
    "model_id": "model-uuid",
    "inputs": {
      "input": {
        "dtype": "float32",
        "shape": [1, 1, 64, 64],
        "data": "base64-encoded-tensor-data"
      }
    }
  }'
```

## Example: Asynchronous Inference

### Using CLI (Recommended)

```bash
# Submit async task
./target/release/ferrinx infer async \
  --name hanzi-tiny --version 1.0 \
  --image ./image.jpg --priority high

# Check task status
./target/release/ferrinx task status <task-id>

# List tasks
./target/release/ferrinx task list --status pending

# Cancel a task
./target/release/ferrinx task cancel <task-id>
```

### Using curl (Raw API)

```bash
# Submit task
curl -X POST http://localhost:8080/api/v1/inference \
  -H "Authorization: Bearer frx_sk_..." \
  -H "Content-Type: application/json" \
  -d '{
    "model_id": "model-uuid",
    "inputs": {"input": {"dtype": "float32", "shape": [1, 1, 64, 64], "data": "base64..."}},
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

# Run with SQLite (default)
cargo run --bin ferrinx-api

# Run with custom database URL
export FERRINX_DATABASE_URL="sqlite://./data/ferrinx.db"
cargo run --bin ferrinx-api

# Run with load-dynamic feature (for older glibc systems)
ORT_DYLIB_PATH=/path/to/libonnxruntime.so cargo run --bin ferrinx-api --features load-dynamic
```

## Tech Stack

| Component | Technology |
|-----------|------------|
| Web Framework | axum |
| ONNX Runtime | ort (with download-binaries or load-dynamic) |
| Database | sqlx (PostgreSQL/SQLite) |
| Cache/Queue | redis (Redis Streams) |
| Async Runtime | tokio |
| CLI | clap |
| Serialization | serde |
| Logging | tracing |

## Philosophy

Ferrinx is built on these principles:

1. **Simplicity First**: The codebase should be easy to understand and modify. No complex abstractions unless necessary.

2. **Flexibility**: Components are loosely coupled. Don't want Redis? No problem. Prefer SQLite over PostgreSQL? Works great.

3. **Decent Performance**: It's not trying to be the fastest, but it's not slow either. Good enough for most use cases.

4. **Hackable**: Want to add a feature? The code is organized to make that straightforward.

## Deployment

### Simple Setup (Recommended)

Perfect for development, experimentation, or small workloads:
```bash
# Just SQLite - no external dependencies needed
cargo run --bin ferrinx-api
```

### With Redis and Worker (Distributed)

When you need to distribute work across multiple machines or run async inference:

```bash
# Start Redis
redis-server

# Terminal 1: Start API server
cargo run --bin ferrinx-api

# Terminal 2: Start worker (can run multiple instances on same or different machines)
cargo run --bin ferrinx-worker
```

See [docs/WORKER.md](docs/WORKER.md) for complete worker configuration and troubleshooting.

## Security

- API Key authentication with SHA-256 hash storage
- Redis caching with database fallback for resilience
- Rate limiting per API key
- Request size limits
- SQL injection prevention via parameterized queries
- Password hashing with bcrypt

## Acknowledgments

- **Hanzi-tiny** - A lightweight Chinese character recognition model provided by [ulink-deep-learning-club/HanziTiny](https://github.com/ulink-deep-learning-club/HanziTiny)

## License

Apache-2.0

## Implementation Status

### ✅ Completed Features

| Feature | Location | Description |
|---------|----------|-------------|
| **Model Configuration System** | `ferrinx-core/src/model/config.rs` | TOML-based model configuration with preprocessing/postprocessing pipelines |
| **Preprocessing Pipelines** | `ferrinx-core/src/transform/pipeline.rs` | Full preprocessing operations: resize, grayscale, normalize, to_tensor, transpose, squeeze, unsqueeze, reshape, center_crop, pad |
| **Postprocessing Pipelines** | `ferrinx-core/src/transform/pipeline.rs` | Full postprocessing operations: softmax, sigmoid, argmax, top_k, threshold, slice, map_labels, nms |
| **NMS (Non-Maximum Suppression)** | `ferrinx-core/src/transform/pipeline.rs` | Object detection post-processing with IoU-based suppression |
| **Model Routing** | `ferrinx-common/src/redis.rs` + `ferrinx-worker/src/model_reporter.rs` | Worker model status reporting and intelligent task routing (cached → available → error) |
| **Synchronous Inference** | `ferrinx-api/src/handlers/inference.rs` | Low-latency in-process inference with LRU cache and semaphore-based concurrency control |
| **Asynchronous Inference** | `ferrinx-api/src/handlers/inference.rs` + `ferrinx-worker/` | Redis Streams-based task queue with worker pool |
| **API Key Authentication** | `ferrinx-api/src/middleware/auth.rs` | Redis cache with database fallback |
| **Rate Limiting** | `ferrinx-api/src/middleware/rate_limit.rs` | Sliding window and token bucket algorithms with DashMap for lock-free concurrency |
| **Bootstrap Endpoint** | `ferrinx-api/src/handlers/auth.rs` | Initial admin user creation with secure random password |
| **Model Upload/Validation** | `ferrinx-api/src/handlers/model.rs` + `ferrinx-core/src/model/loader.rs` | ONNX model upload with validation (magic number check, graph parsing) |
| **Database Abstraction** | `ferrinx-db/` | Repository pattern with SQLite implementation (PostgreSQL pending) |
| **CLI Client** | `ferrinx-cli/` | Full command-line interface for all operations with e2e tests |
| **Worker Pool** | `ferrinx-worker/` | Independent worker processes with Redis Streams consumption, task recovery, and health checks |
| **Graceful Shutdown** | `ferrinx-api/src/main.rs` + `ferrinx-worker/src/main.rs` | Clean shutdown with cancellation tokens |
| **Int8 Tensor Support** | `ferrinx-core/src/tensor.rs` | Int8 tensor type for quantized models |
| **FerrinxTensor Serialization** | `ferrinx-core/src/tensor.rs` | Standardized tensor serialization with base64 encoding and shape metadata |
| **Security Hardening** | Various | bcrypt password hashing, SQL injection prevention, IP spoofing fix, foreign key enforcement |

### 🟡 Partially Implemented

| Feature | Location | Description | Missing Parts |
|---------|----------|-------------|---------------|
| **Prometheus Metrics** | `ferrinx-api/src/handlers/mod.rs:36` | Basic metrics endpoint returning cache/concurrency status | Full Prometheus metrics: request counters, latency histograms, cache hit/miss counters |
| **Transaction Support** | `ferrinx-db/src/context.rs` | Basic transaction begin/commit | Transaction methods (`save_tx`, `delete_tx`, `delete_by_user_tx`) not implemented in repositories |
| **Database Backends** | `ferrinx-db/` | SQLite fully implemented | PostgreSQL implementation pending |
| **GPU Execution Providers** | `ferrinx-core/src/inference/engine.rs` | CPU, CUDA, TensorRT, CoreML, ROCm all implemented via ort 2.0 API | Only CoreML and CUDA tested; TensorRT/ROCm need GPU hardware testing |

### ❌ Not Started

#### Medium Priority

| Feature | Location | Description |
|---------|----------|-------------|
| **Model Metadata Extraction** | `ferrinx-core/src/model/loader.rs:93` | `opset_version` and `producer_name` always return `None` - need to extract from ONNX model |
| **Dynamic Batching** | Not implemented | `DynamicBatcher` for automatic request batching to improve throughput |
| **Database Transaction Methods** | `ferrinx-db/src/traits.rs` | Transaction-specific repository methods (`save_tx`, `delete_tx`, `delete_by_user_tx`) not defined in traits |
| **Batch Operations** | `ferrinx-db/` | Batch cleanup methods, batch delete/update for efficiency |
| **Query Optimization** | `ferrinx-db/` | Covered queries, connection pool metrics, prepared statement caching |
| **PostgreSQL Backend** | `ferrinx-db/src/backends/` | PostgreSQL-specific repository implementations |

#### Low Priority

| Feature | Location | Description |
|---------|----------|-------------|
| **Lua Scripting** | Not implemented | Custom pre/post-processing via Lua scripts for complex transformations |
| **Model Optimizer** | Not implemented | Quantization (INT8), graph optimization, model compression |
| **Version Aliases** | Not implemented | Model version aliasing (e.g., `production` tag pointing to specific version) |
| **OpenTelemetry** | Not implemented | Distributed tracing with Jaeger/Zipkin/OTLP export |
| **Letterbox Preprocessing** | Not implemented | Aspect-ratio preserving resize with padding for images |
| **Input Preprocessing Cache** | Not implemented | Cache for preprocessed inputs to avoid redundant transformations |
| **Model Warmup** | Not implemented | Preload popular models on startup with configurable warmup strategy |
| **Configuration Hot Reload** | Not implemented | Runtime configuration updates without restart |
| **Audit Logging** | Not implemented | Detailed audit trail for all operations |

## Documentation

- [Architecture Design](design.md) - Detailed architecture and design decisions
