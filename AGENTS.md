# AGENTS.md

Context for AI programming agents working on this project.

## Project Overview

**dark-bloom-manager** is a Rust daemon that supervises the Darkbloom (d-inference) provider on macOS Apple Silicon. It enables Darkbloom only when local OMLX inference is idle, ensuring local usage always takes priority.

### Core Behavior

1. Monitor OMLX API for activity (active requests, loaded models)
2. When OMLX is idle for a configurable threshold, start Darkbloom
3. When OMLX needs resources, stop Darkbloom immediately
4. Track analytics (earnings, uptime, state transitions)
5. Serve a web dashboard for monitoring

## Architecture

```
src/
├── main.rs              # CLI entry point (clap)
├── lib.rs               # Public exports, SystemState enum
├── config/mod.rs        # Configuration with serde, validation
├── daemon/
│   ├── runner.rs        # Main event loop, hot-reload support
│   └── signals.rs       # Unix signal handling
├── omlx/
│   ├── client.rs        # HTTP client with session-based auth
│   └── monitor.rs       # Activity state tracking, idle detection
├── darkbloom/
│   ├── controller.rs    # Process management via `darkbloom` CLI
│   └── mod.rs           # Status/earnings types
├── decision/mod.rs      # State machine logic
├── memory/mod.rs        # System memory queries
├── analytics/mod.rs     # SQLite persistence
└── dashboard/
    ├── server.rs        # Axum web server
    └── static/index.html
```

## Key Concepts

### SystemState

The daemon tracks state as one of:
- `OmlxActive` - OMLX is serving requests or has models loaded
- `OmlxIdle` - OMLX is idle, no models loaded
- `DarkbloomActive` - Darkbloom is running and earning
- `StartingDarkbloom` / `StoppingDarkbloom` - Transition states
- `Unknown` - Initial or error state

### OMLX Authentication

OMLX uses **session-based auth**, not Bearer tokens:
1. POST `/admin/api/login` with `{"api_key": "..."}` 
2. Server returns `Set-Cookie` header
3. Use cookie jar for subsequent requests

### Idle Detection

OMLX is considered idle when:
- API is reachable
- No active requests
- No models loaded (for Darkbloom readiness)
- Either: enough consecutive idle polls OR last request older than threshold

### Decision Engine

`decision/mod.rs` contains pure functions that determine actions based on current state. All state transition logic is tested here.

## Development

### Prerequisites

- macOS (Apple Silicon)
- Rust stable
- OMLX running locally (for integration testing)
- `darkbloom` CLI installed (for full functionality)

### Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build
cargo test               # Run all tests
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all          # Format code
```

### Pre-commit Hook

The repo uses `.githooks/pre-commit`. Enable with:
```bash
git config core.hooksPath .githooks
```

Runs: fmt check, clippy, tests

### CI

GitHub Actions runs on macOS only. All actions must be **pinned to full commit SHAs** (Layr-Labs org requirement).

## Important Notes

### Timeouts

- Model unload operations can take 30+ seconds
- OMLX client uses 60s timeout for unload, 30s for other operations

### Startup Behavior

- Require threshold duration worth of polls before first idle transition
- Prevents premature Darkbloom start on daemon startup

### Configuration

- Config file: `~/.config/dark-bloom-manager/config.toml`
- Supports hot-reload via `config update` command or SIGHUP
- CLI overrides available for most settings

### Testing

- Unit tests in each module (`#[cfg(test)]`)
- Decision engine has comprehensive state transition tests
- No integration tests requiring live OMLX/Darkbloom (yet)

## Common Tasks

### Adding a new config option

1. Add field to struct in `src/config/mod.rs`
2. Add default in the struct's `Default` impl or use `#[serde(default)]`
3. Update validation in `Config::validate()` if needed
4. Update `config update` interactive wizard in `main.rs`

### Adding a new API endpoint

1. Add route in `src/dashboard/server.rs`
2. Add handler function
3. Update `static/index.html` if dashboard needs it

### Modifying state transitions

1. Update logic in `src/decision/mod.rs`
2. Add/update tests for the new behavior
3. Ensure all edge cases are covered

## Conventions

- Use `anyhow::Result` for error handling
- Use `tracing` for logging (not `println!` in library code)
- Prefer `#[derive(Default)]` over manual `impl Default`
- Use struct initialization with `..Default::default()` in tests
- Use `&Path` instead of `&PathBuf` in function signatures
