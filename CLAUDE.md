# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

dtomcat is a decentralized WebAssembly runtime platform written in Rust. It functions as an HTTP gateway that manages and executes WASM modules using the Spin framework, similar to how Tomcat runs Java applications but for WebAssembly. The system uses Redis pub/sub for asynchronous message passing between components.

## Essential Commands

### Build
```bash
# Local development build
cargo build --release

# Docker image build
./build-docker-image.sh
```

### Run
```bash
# Local development (using start script)
./start.sh

# Manual run with environment variables
REDIS_HOST="localhost:6379" \
REDIS_URL="redis://localhost:6379" \
DB_HOST="localhost:5432" \
DB_URL="postgresql://postgres:postgres@localhost:5432/#proto?sslmode=disable" \
RUST_LOG=info cargo run
```

### Lint and Type Check
```bash
# Rust formatting check
cargo fmt --check

# Rust linter
cargo clippy

# Type checking is done automatically during build
cargo check
```

## Architecture Overview

### Core Components

1. **HTTP Gateway (src/main.rs:186-256)**
   - Axum-based web server on port 3000
   - Unified handler for all HTTP methods
   - Protocol-based routing using first path segment

2. **Redis Message Bus**
   - Channels: `gate2vin`, `vin2worker`, `vin2worker:{protocol_name}`
   - Request/response matching via UUID-based cache keys
   - 10-second timeout for response polling

3. **WASM Module Management**
   - Dynamic loading via Spin framework
   - Hash-based versioning in `wasm_files/` directory
   - Template-based configuration generation

### Request Flow

1. HTTP request arrives → Extract protocol name from path
2. Create `InputOutputObject` with unique request_id
3. Publish to Redis channel (method-specific routing)
4. Poll Redis for response using cache key pattern
5. Return response with headers and status code

### Key Patterns

- **Async Message Pattern**: HTTP handling decoupled from WASM execution via Redis
- **Protocol Isolation**: Each protocol gets its own Spin instance and Redis channel
- **Dynamic Configuration**: Spin configs generated from `spin_tmpl.toml` template
- **Environment Variable Protocol**: WASM apps access vars prefixed with `SPIN_ENV_`

### WASM Module Lifecycle

1. **Upload**: `ACTION_UPLOAD_WASM` → Save to `wasm_files/{hash}.wasm`
2. **Upgrade**: `ACTION_UPGRADE_WASM` → Generate config, kill old instance, start new
3. **Execution**: Spin manages runtime with configured triggers and permissions

### Important Design Decisions

- GET requests use protocol-specific channels; other methods use shared channel
- Old WASM instances killed with SIGINT before starting new ones
- Outbound hosts must be explicitly whitelisted in Spin configuration
- Request IDs cached with format: `request:{request_id}` and `response:{request_id}`
- Environment variables passed via `SPIN_ENV_*` and `SPIN_VARIABLE_*` prefixes