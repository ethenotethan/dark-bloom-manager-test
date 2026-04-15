# Dark Bloom Manager Specification

**A Rust daemon that supervises Darkbloom (d-inference) provider, enabling it only when local OMLX inference is idle — with full RAM management and an analytics dashboard.**

## Overview

The manager acts as a control plane between two systems:

1. **OMLX** - Local LLM inference server for personal use (CLI + HTTP API at `localhost:8000`)
2. **Darkbloom** - Decentralized inference network where idle Macs earn by serving models

**Core Principle**: Local usage always takes priority. Darkbloom only runs when OMLX models are not actively serving requests. Both systems use Apple Silicon unified memory (MLX), so models must be fully unloaded before switching.

## Architecture

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                         dark-bloom-manager (daemon)                           │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  ┌───────────────────┐  ┌────────────────────┐  ┌─────────────────────────┐  │
│  │   OMLX Monitor    │  │ Darkbloom Control  │  │    Analytics Store      │  │
│  │                   │  │                    │  │                         │  │
│  │ • CLI: omlx ...   │  │ • CLI: darkbloom   │  │ • SQLite persistence    │  │
│  │ • HTTP API poll   │  │ • Process mgmt    │  │ • Time-series metrics   │  │
│  │ • Idle detection  │  │ • RAM tracking     │  │ • Switch events         │  │
│  └─────────┬─────────┘  └──────────┬─────────┘  └────────────┬────────────┘  │
│            │                       │                          │               │
│            └───────────────┬───────┴──────────────────────────┘               │
│                            ▼                                                  │
│                 ┌─────────────────────┐                                       │
│                 │   Decision Engine   │                                       │
│                 │                     │                                       │
│                 │  State Machine:     │                                       │
│                 │  OMLX_ACTIVE        │                                       │
│                 │  OMLX_IDLE          │                                       │
│                 │  TRANSITIONING      │                                       │
│                 │  DARKBLOOM_ACTIVE   │                                       │
│                 └─────────────────────┘                                       │
│                            │                                                  │
│            ┌───────────────┴───────────────┐                                  │
│            ▼                               ▼                                  │
│  ┌───────────────────┐          ┌───────────────────┐                        │
│  │   Web Dashboard   │          │  Metrics Server   │                        │
│  │   localhost:9090  │          │  /metrics         │                        │
│  └───────────────────┘          └───────────────────┘                        │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
          │                                        │
          ▼                                        ▼
┌───────────────────┐                    ┌───────────────────┐
│       OMLX        │                    │    Darkbloom      │
│                   │                    │                   │
│ CLI:              │                    │ CLI:              │
│ • omlx serve      │                    │ • darkbloom start │
│ • omlx models ... │                    │ • darkbloom stop  │
│                   │                    │ • darkbloom status│
│ HTTP API:         │                    │                   │
│ • /admin/api/*    │                    │ RAM: ~28-243 GB   │
│ • /v1/*           │                    │ (model dependent) │
│                   │                    │                   │
│ RAM: variable     │                    │                   │
└───────────────────┘                    └───────────────────┘
```

## RAM Management

### The Problem

Both OMLX and Darkbloom load LLM weights into Apple Silicon unified memory. Running both simultaneously would cause memory pressure or OOM. The manager enforces **mutual exclusion** at the RAM level.

### Darkbloom RAM Requirements

Based on Darkbloom's model catalog:

| Model | Size | Min RAM |
|-------|------|---------|
| Gemma 4 26B 8-bit | 28 GB | 36 GB |
| Qwen3.5 27B 8-bit | 27 GB | 36 GB |
| Trinity Mini 8-bit | 26 GB | 48 GB |
| Qwen3.5 122B MoE 4-bit | 122 GB | 128 GB |
| MiniMax M2.5 8-bit | 243 GB | 256 GB |
| FLUX.2 Klein 4B | 8.1 GB | 16 GB |
| FLUX.2 Klein 9B | 13 GB | 24 GB |

### OMLX RAM Requirements

OMLX loads MLX models dynamically. RAM usage depends on loaded models:
- Typical coding models: 8-32 GB
- Large models (122B+): 64-128 GB

### Transition Sequence

**OMLX → Darkbloom** (when OMLX goes idle):
```
1. Detect OMLX idle (no requests for threshold period)
2. Query OMLX loaded models: GET /admin/api/models
3. Unload all OMLX models via: POST /admin/api/models/{id}/unload (for each)
4. Verify RAM freed (poll system memory or re-check /admin/api/models)
5. Start Darkbloom: `darkbloom start`
6. Wait for Darkbloom healthy: `darkbloom status`
7. Log transition, record analytics
```

**Darkbloom → OMLX** (when local request arrives):
```
1. Detect incoming OMLX activity (API request or explicit trigger)
2. Signal Darkbloom graceful shutdown (wait for in-flight request)
3. Stop Darkbloom: `darkbloom stop`
4. Verify Darkbloom stopped and RAM freed
5. OMLX auto-loads models on next request (LRU cache)
6. Log transition, record analytics
```

### OMLX Model Control

OMLX CLI has two main commands:
```bash
omlx serve --model-dir ~/models    # Start multi-model server
omlx launch <tool>                 # Launch integrated tools (codex, opencode, etc.)
```

Model management is done exclusively via the **Admin HTTP API** (no CLI commands for model control):

```
# List all models with status
GET http://localhost:8000/admin/api/models

# Unload a specific model from memory
POST http://localhost:8000/admin/api/models/{model_id}/unload

# Load a specific model into memory  
POST http://localhost:8000/admin/api/models/{model_id}/load

# Reload all models (re-scan directories, re-apply settings)
POST http://localhost:8000/admin/api/reload

# Get server stats (memory, active requests, etc.)
GET http://localhost:8000/admin/api/stats
```

**Note**: OMLX uses LRU eviction and process memory enforcement for automatic model management. The manager should unload models explicitly before starting Darkbloom to ensure RAM is freed.

### RAM Verification

Before starting the other system, verify memory is actually freed:

```rust
struct MemoryState {
    system_total_gb: f64,
    system_used_gb: f64,
    system_available_gb: f64,
    omlx_reported_gb: Option<f64>,
    darkbloom_reported_gb: Option<f64>,
}

impl MemoryState {
    fn safe_for_darkbloom(&self, model: &DarkbloomModel) -> bool {
        self.system_available_gb >= model.min_ram_gb
    }
}
```

## Web Dashboard

### Endpoint

```
http://localhost:9090/dashboard
```

### Features

1. **Live Status Panel**
   - Current state: OMLX Active / Darkbloom Active / Transitioning
   - Active system uptime
   - Current RAM usage (gauge)
   - Loaded models list

2. **Activity Timeline**
   - Visual timeline of OMLX vs Darkbloom active periods
   - Zoom: hour / day / week / month

3. **Analytics Cards**
   - OMLX usage: requests/hour, tokens processed, active time %
   - Darkbloom usage: requests served, earnings estimate, active time %
   - Switch frequency: transitions/day
   - Idle time: % of time neither system active

4. **Switching History**
   - Table of all transitions with timestamps
   - Transition duration (time to unload + load)
   - Trigger reason (idle timeout, manual, incoming request)

5. **RAM History**
   - Time-series chart of memory usage
   - Peak usage markers
   - Model load/unload events annotated

6. **Configuration Panel**
   - Edit idle threshold
   - Enable/disable manager
   - Manual override: force OMLX / force Darkbloom / auto

### API Endpoints

```
GET  /api/status           # Current state
GET  /api/analytics        # Aggregated stats
GET  /api/timeline?range=  # Activity timeline data
GET  /api/switches         # Switch history
GET  /api/memory           # Memory time-series
POST /api/config           # Update configuration
POST /api/override         # Manual state override
```

### Tech Stack

- **Backend**: Axum (Rust) serving JSON API + static files
- **Frontend**: Embedded SPA (Preact + lightweight charts)
- **Storage**: SQLite for analytics persistence

## OMLX Activity Detection

### Primary Method: HTTP API Polling

```
GET http://localhost:8000/admin/api/stats
GET http://localhost:8000/admin/api/models
```

The `/admin/api/models` endpoint returns detailed model status:
```json
{
  "models": [
    {
      "id": "qwen3-8b",
      "loaded": true,
      "is_loading": false,
      "estimated_size": 8589934592,
      "estimated_size_formatted": "8.00 GB",
      "pinned": false,
      "model_type": "llm",
      "last_access": "2026-04-16T10:30:00Z"
    }
  ]
}
```

The `/admin/api/stats` endpoint returns server-wide metrics:
```json
{
  "active_requests": 0,
  "total_requests": 1234,
  "memory_used_gb": 24.5,
  "uptime_secs": 3600
}
```

### Secondary: Process Monitoring

If API unavailable, fall back to process inspection:
- Check if `omlx` process has recent CPU activity
- Monitor network connections to port 8000

### Idle Detection Algorithm

```rust
#[derive(Debug, Clone)]
struct OmlxActivityState {
    last_request_time: Option<DateTime<Utc>>,
    active_request_count: u32,
    loaded_models: Vec<String>,
    memory_used_gb: f64,
    consecutive_idle_polls: u32,
    api_reachable: bool,
}

impl OmlxActivityState {
    fn is_idle(&self, config: &Config) -> bool {
        // Must have zero active requests
        if self.active_request_count > 0 {
            return false;
        }
        
        // Must have been idle for threshold duration
        let idle_long_enough = self.last_request_time
            .map(|t| Utc::now() - t > config.idle_threshold)
            .unwrap_or(true);
        
        // Must have consistent idle readings
        let stable = self.consecutive_idle_polls >= config.min_idle_polls;
        
        idle_long_enough && stable
    }
    
    fn should_unload_models(&self) -> bool {
        self.is_idle() && !self.loaded_models.is_empty()
    }
}
```

## Darkbloom Control

### CLI Interface

```bash
darkbloom start              # Start as background daemon
darkbloom stop               # Stop daemon (graceful, waits for in-flight)
darkbloom status             # JSON status output
darkbloom status --json      # Explicit JSON format

# Output of `darkbloom status`:
{
  "running": true,
  "connected": true,
  "model": "qwen3.5-27b-claude-opus-8bit",
  "active_request": false,
  "uptime_secs": 3600,
  "requests_served": 42,
  "earnings_usd": 0.15
}
```

### State Machine

```
                        ┌──────────────┐
                        │  OMLX_ACTIVE │◄─────────────────────┐
                        │              │                      │
                        │ Models loaded│                      │
                        │ Serving local│                      │
                        └──────┬───────┘                      │
                               │                              │
                               │ Idle threshold reached       │
                               ▼                              │
                    ┌─────────────────────┐                   │
                    │ UNLOADING_OMLX      │                   │
                    │                     │                   │
                    │ • Unload all models │                   │
                    │ • Verify RAM freed  │                   │ OMLX request
                    └──────────┬──────────┘                   │ detected
                               │                              │
                               │ RAM available                │
                               ▼                              │
                    ┌─────────────────────┐                   │
                    │ STARTING_DARKBLOOM  │                   │
                    │                     │                   │
                    │ • darkbloom start   │───► Timeout ──────┤
                    │ • Wait healthy      │                   │
                    └──────────┬──────────┘                   │
                               │                              │
                               │ Darkbloom healthy            │
                               ▼                              │
                    ┌─────────────────────┐                   │
                    │ DARKBLOOM_ACTIVE    │                   │
                    │                     │                   │
                    │ Serving d-inference │                   │
                    │ Earning revenue     │                   │
                    └──────────┬──────────┘                   │
                               │                              │
                               │ OMLX request OR manual       │
                               ▼                              │
                    ┌─────────────────────┐                   │
                    │ STOPPING_DARKBLOOM  │                   │
                    │                     │                   │
                    │ • Wait in-flight    │                   │
                    │ • darkbloom stop    │                   │
                    │ • Verify stopped    │                   │
                    └──────────┬──────────┘                   │
                               │                              │
                               │ Darkbloom stopped            │
                               └──────────────────────────────┘
```

### Graceful Shutdown Handling

When stopping Darkbloom:

```rust
async fn stop_darkbloom(&self, strategy: ShutdownStrategy) -> Result<()> {
    // 1. Check if mid-inference
    let status = self.get_darkbloom_status().await?;
    
    if status.active_request {
        match strategy {
            ShutdownStrategy::Immediate => {
                // Force stop, may interrupt
                self.exec_darkbloom_stop().await?;
            }
            ShutdownStrategy::Graceful { timeout } => {
                // Wait for request to complete
                let deadline = Instant::now() + timeout;
                while Instant::now() < deadline {
                    let status = self.get_darkbloom_status().await?;
                    if !status.active_request {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                self.exec_darkbloom_stop().await?;
            }
        }
    } else {
        self.exec_darkbloom_stop().await?;
    }
    
    // 2. Verify stopped
    self.wait_for_stopped(Duration::from_secs(30)).await?;
    
    Ok(())
}
```

## Analytics & Persistence

### SQLite Schema

```sql
-- State transitions
CREATE TABLE transitions (
    id INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    trigger TEXT NOT NULL,        -- 'idle_timeout', 'omlx_request', 'manual'
    duration_ms INTEGER,          -- Time to complete transition
    success BOOLEAN NOT NULL
);

-- Periodic snapshots (every minute)
CREATE TABLE snapshots (
    id INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL,
    state TEXT NOT NULL,
    omlx_loaded_models TEXT,      -- JSON array
    omlx_memory_gb REAL,
    darkbloom_model TEXT,
    darkbloom_connected BOOLEAN,
    system_memory_available_gb REAL
);

-- OMLX request log (sampled)
CREATE TABLE omlx_requests (
    id INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL,
    model TEXT,
    tokens_in INTEGER,
    tokens_out INTEGER,
    duration_ms INTEGER
);

-- Darkbloom earnings
CREATE TABLE darkbloom_sessions (
    id INTEGER PRIMARY KEY,
    start_time TEXT NOT NULL,
    end_time TEXT,
    model TEXT,
    requests_served INTEGER DEFAULT 0,
    earnings_usd REAL DEFAULT 0
);

-- Indexes
CREATE INDEX idx_transitions_timestamp ON transitions(timestamp);
CREATE INDEX idx_snapshots_timestamp ON snapshots(timestamp);
```

### Aggregated Metrics

```rust
struct AnalyticsSummary {
    period: TimePeriod,  // Hour, Day, Week, Month
    
    // Time allocation
    omlx_active_pct: f64,
    darkbloom_active_pct: f64,
    idle_pct: f64,
    transitioning_pct: f64,
    
    // OMLX stats
    omlx_requests: u64,
    omlx_tokens_processed: u64,
    
    // Darkbloom stats
    darkbloom_requests_served: u64,
    darkbloom_earnings_usd: f64,
    
    // Switching
    transitions_count: u32,
    avg_transition_duration_ms: u64,
    
    // Memory
    peak_memory_gb: f64,
    avg_memory_gb: f64,
}
```

## Configuration

Location: `~/.config/dark-bloom-manager/config.toml`

```toml
[daemon]
log_level = "info"
data_dir = "~/.local/share/dark-bloom-manager"

[dashboard]
enabled = true
port = 9090
# Optional: bind to specific address
# bind = "127.0.0.1"

[omlx]
endpoint = "http://localhost:8000"
poll_interval_secs = 5
idle_threshold_secs = 60          # Idle for 60s before switching
min_idle_polls = 3                # Require 3 consecutive idle readings
request_timeout_secs = 5
# Behavior when OMLX API unreachable
unreachable_behavior = "assume_active"  # or "assume_idle"

[darkbloom]
binary_path = "darkbloom"         # Or absolute path
model = "qwen3.5-27b-claude-opus-8bit"  # Default model to serve
startup_timeout_secs = 60
shutdown_timeout_secs = 120       # Wait for graceful shutdown
shutdown_strategy = "graceful"    # or "immediate"
# RAM required for the configured model
model_ram_gb = 36

[memory]
# Minimum available RAM before starting Darkbloom
min_available_gb = 40
# Poll interval for memory checks during transitions  
check_interval_secs = 2

[analytics]
enabled = true
snapshot_interval_secs = 60
retention_days = 90               # Delete old data after 90 days
```

## CLI Interface

```
dark-bloom-manager 0.1.0
Supervisor for Darkbloom provider with OMLX coordination

USAGE:
    dark-bloom-manager <COMMAND>

COMMANDS:
    run           Run the daemon
    install       Install as launchd service
    uninstall     Remove launchd service
    start         Start the launchd service
    stop          Stop the launchd service
    status        Show current status (JSON)
    dashboard     Open dashboard in browser
    logs          Tail daemon logs
    analytics     Show analytics summary
    config        Show/edit configuration
    help          Print help

RUN OPTIONS:
    --foreground  Run in foreground (don't daemonize)
    --debug       Enable debug logging

ANALYTICS OPTIONS:
    --period <PERIOD>  Hour, day, week, month [default: day]
    --json             Output as JSON

EXAMPLES:
    dark-bloom-manager run --foreground
    dark-bloom-manager status
    dark-bloom-manager analytics --period week
    dark-bloom-manager config --edit
```

### Status Output

```json
{
  "daemon": {
    "running": true,
    "uptime_secs": 86400,
    "pid": 12345,
    "version": "0.1.0"
  },
  "state": "DARKBLOOM_ACTIVE",
  "omlx": {
    "api_reachable": true,
    "loaded_models": [],
    "memory_gb": 0,
    "last_request": "2026-04-16T10:30:00Z",
    "idle_duration_secs": 3600
  },
  "darkbloom": {
    "running": true,
    "connected": true,
    "model": "qwen3.5-27b-claude-opus-8bit",
    "uptime_secs": 3500,
    "requests_served": 12,
    "active_request": false
  },
  "memory": {
    "system_total_gb": 128,
    "system_available_gb": 92,
    "estimated_darkbloom_gb": 36
  },
  "analytics": {
    "today": {
      "omlx_active_pct": 45.2,
      "darkbloom_active_pct": 48.1,
      "transitions": 8
    }
  }
}
```

## Project Structure

```
dark-bloom-manager/
├── Cargo.toml
├── src/
│   ├── main.rs                 # CLI entry point (clap)
│   ├── lib.rs                  # Library root
│   │
│   ├── daemon/
│   │   ├── mod.rs
│   │   ├── runner.rs           # Main event loop
│   │   ├── signals.rs          # SIGTERM, SIGHUP handling
│   │   └── state.rs            # State machine
│   │
│   ├── omlx/
│   │   ├── mod.rs
│   │   ├── client.rs           # HTTP client for OMLX API
│   │   ├── models.rs           # Model load/unload operations
│   │   └── monitor.rs          # Activity detection
│   │
│   ├── darkbloom/
│   │   ├── mod.rs
│   │   ├── cli.rs              # CLI wrapper (start/stop/status)
│   │   ├── process.rs          # Process management
│   │   └── models.rs           # Model catalog & RAM info
│   │
│   ├── memory/
│   │   ├── mod.rs
│   │   └── macos.rs            # macOS memory stats (sysctl)
│   │
│   ├── decision/
│   │   ├── mod.rs
│   │   └── engine.rs           # Transition logic
│   │
│   ├── analytics/
│   │   ├── mod.rs
│   │   ├── store.rs            # SQLite operations
│   │   ├── aggregator.rs       # Compute summaries
│   │   └── recorder.rs         # Event recording
│   │
│   ├── dashboard/
│   │   ├── mod.rs
│   │   ├── server.rs           # Axum HTTP server
│   │   ├── api.rs              # JSON API handlers
│   │   └── static/             # Embedded frontend assets
│   │       ├── index.html
│   │       ├── app.js
│   │       └── style.css
│   │
│   ├── config/
│   │   ├── mod.rs
│   │   └── schema.rs           # Config structs + validation
│   │
│   └── launchd/
│       └── mod.rs              # plist generation, launchctl
│
├── migrations/
│   └── 001_initial.sql         # SQLite schema
│
├── tests/
│   ├── integration/
│   │   ├── daemon_test.rs
│   │   └── transitions_test.rs
│   └── unit/
│
├── dashboard-ui/               # Optional: separate frontend build
│   ├── package.json
│   ├── src/
│   └── dist/                   # Built assets copied to src/dashboard/static/
│
└── README.md
```

## Dependencies

```toml
[package]
name = "dark-bloom-manager"
version = "0.1.0"
edition = "2021"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# HTTP client (for OMLX API)
reqwest = { version = "0.12", features = ["json"] }

# HTTP server (dashboard)
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["fs", "cors"] }

# Database
rusqlite = { version = "0.32", features = ["bundled"] }

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Error handling
thiserror = "2"
anyhow = "1"

# Utilities
directories = "5"               # XDG/macOS paths
chrono = { version = "0.4", features = ["serde"] }
nix = { version = "0.29", features = ["signal", "process"] }
sysinfo = "0.32"                # System memory info

# Embed static files
rust-embed = "8"

[dev-dependencies]
mockall = "0.13"
wiremock = "0.6"
assert_cmd = "2"
predicates = "3"
tempfile = "3"
tokio-test = "0.4"
```

## Error Handling

### OMLX Unreachable

```rust
enum OmlxUnreachableBehavior {
    AssumeActive,   // Safe: don't start Darkbloom
    AssumeIdle,     // Aggressive: start Darkbloom anyway
}
```

Default: `AssumeActive` (fail-safe)

### Transition Failures

| Failure | Recovery |
|---------|----------|
| OMLX model unload fails | Retry 3x, then abort transition |
| Darkbloom start timeout | Log error, stay in OMLX_ACTIVE |
| Darkbloom stop timeout | SIGKILL after grace period |
| RAM not freed | Wait longer, then force GC hint |

### Signal Handling

| Signal | Action |
|--------|--------|
| SIGTERM | Graceful shutdown (stop Darkbloom first if running) |
| SIGINT | Same as SIGTERM |
| SIGHUP | Reload configuration |
| SIGUSR1 | Dump state to log |

## Future Enhancements

1. **OMLX request interception** - Proxy mode to catch requests before they hit OMLX, enabling faster Darkbloom→OMLX transitions
2. **Multi-model awareness** - Track which specific models are needed and only unload conflicting ones
3. **Earnings tracking** - Pull actual earnings from Darkbloom API when available
4. **macOS notifications** - Alert on state changes, earnings milestones
5. **Menu bar app** - Native Swift companion showing status in menu bar
6. **Scheduling integration** - Coordinate with Darkbloom's `provider.toml` time windows
7. **Remote dashboard** - Optional authenticated access from other devices
