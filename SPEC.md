# Dark Bloom Manager Specification

**A Rust daemon that supervises Darkbloom (d-inference) provider, enabling it only when local OMLX inference is idle.**

## Overview

The manager acts as a control plane between two systems:

1. **OMLX** - Local LLM inference server for personal use (`http://localhost:8000/v1`)
2. **Darkbloom** - Decentralized inference network where idle Macs earn by serving models

**Core Principle**: Local usage always takes priority. Darkbloom only runs when OMLX models are not actively serving requests.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    dark-bloom-manager (daemon)                   │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────────┐    ┌──────────────────┐                   │
│  │  OMLX Monitor    │    │ Darkbloom Control│                   │
│  │                  │    │                  │                   │
│  │ - Poll /v1/stats │    │ - start/stop CLI │                   │
│  │ - Track activity │    │ - health checks  │                   │
│  │ - Idle detection │    │ - state machine  │                   │
│  └────────┬─────────┘    └────────┬─────────┘                   │
│           │                       │                              │
│           └───────────┬───────────┘                              │
│                       ▼                                          │
│              ┌────────────────┐                                  │
│              │ Decision Engine│                                  │
│              │                │                                  │
│              │ OMLX idle >    │                                  │
│              │ threshold? ────┼──► Start Darkbloom               │
│              │                │                                  │
│              │ OMLX active? ──┼──► Stop Darkbloom (graceful)     │
│              └────────────────┘                                  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
         │                                    │
         ▼                                    ▼
┌─────────────────┐                 ┌─────────────────┐
│     OMLX        │                 │   Darkbloom     │
│  localhost:8000 │                 │   Provider      │
│                 │                 │                 │
│ GET /v1/models  │                 │ darkbloom start │
│ GET /admin/api/ │                 │ darkbloom stop  │
│   stats         │                 │ darkbloom status│
└─────────────────┘                 └─────────────────┘
```

## OMLX Activity Detection

### Primary Method: API Polling

OMLX exposes an admin API. Poll for activity metrics:

```
GET http://localhost:8000/admin/api/stats
GET http://localhost:8000/admin/api/engines  # loaded models & active requests
```

### Activity Indicators

| Metric | Idle Condition |
|--------|----------------|
| Active requests | 0 |
| Requests in last N seconds | 0 |
| Loaded models with recent activity | None |

### Idle Detection Algorithm

```rust
struct ActivityState {
    last_request_time: Option<Instant>,
    active_request_count: u32,
    consecutive_idle_polls: u32,
}

impl ActivityState {
    fn is_idle(&self, config: &Config) -> bool {
        self.active_request_count == 0 
            && self.last_request_time
                .map(|t| t.elapsed() > config.idle_threshold)
                .unwrap_or(true)
            && self.consecutive_idle_polls >= config.min_idle_polls
    }
}
```

### Configuration

```toml
[omlx]
endpoint = "http://localhost:8000"
poll_interval_secs = 5
idle_threshold_secs = 60      # No requests for 60s = idle
min_idle_polls = 3            # Require 3 consecutive idle polls
request_timeout_secs = 5
```

## Darkbloom Control

### CLI Interface

```bash
darkbloom start          # Start as background daemon
darkbloom stop           # Stop daemon (graceful)
darkbloom status         # Get current state (JSON parseable)
darkbloom serve          # Foreground mode (for debugging)
```

### State Machine

```
                    ┌─────────┐
                    │ UNKNOWN │ (initial)
                    └────┬────┘
                         │ status check
                         ▼
        ┌────────────────┴────────────────┐
        │                                 │
        ▼                                 ▼
   ┌─────────┐                      ┌──────────┐
   │ STOPPED │ ◄────── stop ─────── │ RUNNING  │
   └────┬────┘                      └────┬─────┘
        │                                │
        │ OMLX idle                      │ OMLX active
        │                                │
        └──────► start ──────────────────┘
                         │
                         ▼
                   ┌───────────┐
                   │ STARTING  │ (wait for healthy)
                   └───────────┘
```

### Graceful Shutdown

When OMLX becomes active:

1. Check if Darkbloom is mid-inference (serving a request)
2. If yes: wait for current request to complete (with timeout)
3. Send `darkbloom stop`
4. Verify shutdown via `darkbloom status`
5. Retry with SIGTERM/SIGKILL if unresponsive

```rust
enum ShutdownStrategy {
    Immediate,           // Stop now, interrupt if needed
    GracefulWithTimeout(Duration),  // Wait for current request, then stop
}
```

### Configuration

```toml
[darkbloom]
binary_path = "darkbloom"           # or absolute path
startup_timeout_secs = 30
shutdown_timeout_secs = 60          # Wait for graceful shutdown
health_check_interval_secs = 10
max_restart_attempts = 3
restart_backoff_secs = 30
```

## Daemon Behavior

### Lifecycle

```bash
# Install as launchd service
dark-bloom-manager install

# Manual control
dark-bloom-manager start
dark-bloom-manager stop
dark-bloom-manager status

# Run in foreground (for debugging)
dark-bloom-manager run --foreground
```

### Launchd Integration (macOS)

Generate plist at `~/Library/LaunchAgents/ai.darkbloom.manager.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "...">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.darkbloom.manager</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/dark-bloom-manager</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>~/.local/share/dark-bloom-manager/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>~/.local/share/dark-bloom-manager/daemon.err</string>
</dict>
</plist>
```

### Logging

- Structured logging with `tracing`
- Log levels: ERROR, WARN, INFO, DEBUG, TRACE
- Log rotation via external tool or built-in size limit
- Log location: `~/.local/share/dark-bloom-manager/logs/`

### Health & Metrics

Expose optional metrics endpoint:

```
GET http://localhost:9090/health
GET http://localhost:9090/metrics  # Prometheus format
```

Metrics:
- `omlx_last_activity_timestamp`
- `omlx_idle_duration_seconds`
- `darkbloom_state` (0=stopped, 1=running, 2=starting)
- `darkbloom_uptime_seconds`
- `manager_decisions_total{action="start|stop"}`

## Configuration File

Location: `~/.config/dark-bloom-manager/config.toml`

```toml
[daemon]
log_level = "info"
metrics_enabled = true
metrics_port = 9090

[omlx]
endpoint = "http://localhost:8000"
poll_interval_secs = 5
idle_threshold_secs = 60
min_idle_polls = 3
request_timeout_secs = 5

[darkbloom]
binary_path = "darkbloom"
startup_timeout_secs = 30
shutdown_timeout_secs = 60
shutdown_strategy = "graceful"  # or "immediate"
health_check_interval_secs = 10

[schedule]
# Optional: Only manage during certain hours (defer to darkbloom's own scheduling)
enabled = false
# windows = [
#   { days = ["mon", "tue", "wed", "thu", "fri"], start = "22:00", end = "08:00" }
# ]
```

## CLI Interface

```
dark-bloom-manager 0.1.0
Supervisor daemon for Darkbloom provider, activated when OMLX is idle

USAGE:
    dark-bloom-manager <COMMAND>

COMMANDS:
    run         Run the daemon (foreground or background)
    install     Install as launchd service
    uninstall   Remove launchd service
    start       Start the launchd service
    stop        Stop the launchd service  
    status      Show current status (JSON)
    config      Show or validate configuration
    help        Print help

OPTIONS:
    -c, --config <PATH>    Config file path
    -v, --verbose          Increase verbosity (-v, -vv, -vvv)
    -q, --quiet            Suppress output
    -h, --help             Print help
    -V, --version          Print version
```

### Status Output

```json
{
  "daemon": {
    "running": true,
    "uptime_secs": 3600,
    "pid": 12345
  },
  "omlx": {
    "reachable": true,
    "idle": true,
    "idle_duration_secs": 120,
    "last_request": "2026-04-16T10:30:00Z"
  },
  "darkbloom": {
    "state": "running",
    "managed": true,
    "uptime_secs": 100,
    "pid": 12346
  }
}
```

## Error Handling

### OMLX Unreachable

- Treat as "potentially active" (fail-safe: don't start Darkbloom)
- Retry with exponential backoff
- Log warning after N consecutive failures
- Optional: Start Darkbloom after extended OMLX downtime (configurable)

```toml
[omlx]
unreachable_action = "assume_active"  # or "assume_idle" or "start_darkbloom_after_secs"
unreachable_timeout_secs = 300        # If assume_idle after timeout
```

### Darkbloom Failures

- Track consecutive start failures
- Implement backoff: 30s, 60s, 120s, 300s
- Alert/log after max retries exceeded
- Auto-recover on next successful health check

### Signal Handling

| Signal | Action |
|--------|--------|
| SIGTERM | Graceful shutdown (stop Darkbloom first) |
| SIGINT | Graceful shutdown |
| SIGHUP | Reload configuration |
| SIGUSR1 | Dump current state to log |

## Edge Cases

### Race Conditions

1. **OMLX request arrives during Darkbloom startup**
   - Cancel startup, don't start Darkbloom
   
2. **Darkbloom mid-inference when OMLX request arrives**
   - Wait for Darkbloom request to complete (with timeout)
   - Then gracefully stop Darkbloom

3. **Manager restart while Darkbloom is running**
   - On startup, check `darkbloom status`
   - Adopt existing Darkbloom process into managed state

### Resource Contention

Both OMLX and Darkbloom use GPU memory. The manager ensures mutual exclusion:

```
OMLX active  → Darkbloom must be stopped
OMLX idle    → Darkbloom may start
Darkbloom running + OMLX request → Stop Darkbloom first? (configurable)
```

```toml
[behavior]
# If OMLX gets a request while Darkbloom is running:
on_omlx_request = "stop_darkbloom"  # or "let_omlx_queue" (if OMLX can queue)
```

## Project Structure

```
dark-bloom-manager/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI entry point
│   ├── lib.rs               # Library root
│   ├── daemon/
│   │   ├── mod.rs
│   │   ├── runner.rs        # Main daemon loop
│   │   └── signals.rs       # Signal handling
│   ├── omlx/
│   │   ├── mod.rs
│   │   ├── client.rs        # HTTP client for OMLX API
│   │   └── monitor.rs       # Activity monitoring logic
│   ├── darkbloom/
│   │   ├── mod.rs
│   │   ├── controller.rs    # Start/stop/status
│   │   └── process.rs       # Process management
│   ├── decision/
│   │   ├── mod.rs
│   │   └── engine.rs        # Decision logic
│   ├── config/
│   │   ├── mod.rs
│   │   └── schema.rs        # Config structs
│   ├── metrics/
│   │   └── mod.rs           # Prometheus metrics
│   └── launchd/
│       └── mod.rs           # macOS service management
├── tests/
│   ├── integration/
│   └── unit/
└── README.md
```

## Dependencies (Cargo.toml)

```toml
[package]
name = "dark-bloom-manager"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
reqwest = { version = "0.12", features = ["json"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "2"
anyhow = "1"
directories = "5"           # XDG paths
nix = { version = "0.29", features = ["signal", "process"] }
metrics = "0.24"
metrics-exporter-prometheus = "0.16"

[dev-dependencies]
mockall = "0.13"
wiremock = "0.6"
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```

## Future Enhancements

1. **Multiple OMLX instances** - Monitor multiple local inference servers
2. **Model-aware scheduling** - Only start Darkbloom if its model doesn't conflict
3. **Resource-based decisions** - Factor in GPU memory, not just request activity
4. **Web dashboard** - Simple status page
5. **Notifications** - macOS notifications for state changes
6. **Integration with Darkbloom scheduling** - Coordinate with `provider.toml` windows
