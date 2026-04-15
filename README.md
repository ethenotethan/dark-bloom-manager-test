# Dark Bloom Manager

**Supervisor daemon that maximizes your Mac's earning potential by running [Darkbloom](https://github.com/Layr-Labs/d-inference) inference only when your local [OMLX](https://github.com/jundot/omlx) server is idle.**

```
┌─────────────────────────────────────────────────────────────┐
│                    Your Mac's GPU Memory                     │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   OMLX Active (you're coding)     Darkbloom Active (idle)   │
│   ████████████████░░░░░░░░░░░░    ░░░░░░░░████████████████  │
│   ← Local inference priority      Earn from idle compute →  │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Why?

- **OMLX** runs local LLM inference for your coding tools (Claude Code, Cursor, etc.)
- **Darkbloom** lets you earn by serving decentralized inference when idle
- **Both use GPU memory** — they can't run simultaneously
- **This daemon** automatically switches between them, maximizing utilization

## Features

- **Automatic switching** — Detects OMLX idle state and starts Darkbloom
- **RAM management** — Unloads OMLX models before starting Darkbloom
- **Graceful transitions** — Waits for in-flight requests before switching
- **Live dashboard** — Real-time status, memory charts, activity timeline
- **Earnings tracking** — Track daily/weekly/monthly earnings and session history
- **macOS integration** — Runs as a launchd service with auto-restart

## Installation

### From Source

```bash
git clone https://github.com/Layr-Labs/dark-bloom-manager.git
cd dark-bloom-manager
cargo build --release

# Copy to PATH
cp target/release/dark-bloom-manager /usr/local/bin/
```

### Prerequisites

- macOS with Apple Silicon (M1/M2/M3/M4)
- [OMLX](https://github.com/jundot/omlx) installed and running
- [Darkbloom](https://github.com/Layr-Labs/d-inference) CLI installed

## Quick Start

```bash
# Interactive setup wizard (first time)
dark-bloom-manager config init

# Or configure via CLI flags
dark-bloom-manager run --foreground \
  --omlx-endpoint http://localhost:8000 \
  --omlx-api-key your-api-key \
  --idle-threshold 60

# Or install as a background service
dark-bloom-manager install
dark-bloom-manager start

# Open the dashboard
dark-bloom-manager dashboard
```

## CLI Commands

```
dark-bloom-manager [OPTIONS] <COMMAND>

Commands:
  run         Run the daemon (--foreground for debug)
  install     Install as launchd service
  uninstall   Remove launchd service
  start       Start the launchd service
  stop        Stop the launchd service
  status      Show current status (--json for JSON output)
  dashboard   Open dashboard in browser
  analytics   Show analytics summary (--period hour|day|week|month)
  config      Manage configuration (init, set, get, show, edit)

Global Options:
  --omlx-endpoint <URL>     OMLX server endpoint (env: OMLX_ENDPOINT)
  --omlx-port <PORT>        OMLX server port
  --omlx-api-key <KEY>      OMLX API key (env: OMLX_API_KEY)
  --idle-threshold <SECS>   Seconds before switching to Darkbloom
  --darkbloom-binary <PATH> Path to darkbloom binary
  --darkbloom-model <NAME>  Darkbloom model to serve
  --darkbloom-model-ram <GB> RAM required for model
  --dashboard-port <PORT>   Dashboard server port
  --no-dashboard            Disable dashboard server
  --min-memory <GB>         Minimum available memory for Darkbloom
  -c, --config <PATH>       Custom config file path
  -v, --verbose             Increase verbosity (-v, -vv, -vvv)
```

### Configuration Commands

```bash
# Interactive setup wizard
dark-bloom-manager config init

# Set individual values
dark-bloom-manager config set omlx.endpoint http://localhost:8000
dark-bloom-manager config set omlx.api_key your-secret-key
dark-bloom-manager config set omlx.idle_threshold 120
dark-bloom-manager config set darkbloom.model qwen3.5-27b-claude-opus-8bit

# Get a value
dark-bloom-manager config get omlx.endpoint

# Show full config
dark-bloom-manager config show

# Open in editor
dark-bloom-manager config edit

# Validate config
dark-bloom-manager config validate

# Show config file path
dark-bloom-manager config path
```

### Examples

```bash
# Run with custom OMLX settings
dark-bloom-manager run --foreground \
  --omlx-endpoint http://localhost:8000 \
  --omlx-api-key sk-xxx \
  --idle-threshold 120

# Use environment variables
export OMLX_API_KEY=sk-xxx
export OMLX_ENDPOINT=http://localhost:8000
dark-bloom-manager run --foreground

# Check current status
dark-bloom-manager status

# View weekly analytics
dark-bloom-manager analytics --period week
```

## Dashboard

Access at `http://localhost:9090/dashboard` when the daemon is running.

```
┌─────────────────────────────────────────────────────────────┐
│  Dark Bloom Manager                          ● Connected    │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Current State: Darkbloom Active    Uptime: 02:45:30│   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐                     │
│  │  OMLX   │  │Darkbloom│  │ Memory  │                     │
│  │ Online  │  │ Running │  │ 92/128GB│                     │
│  └─────────┘  └─────────┘  └─────────┘                     │
│                                                             │
│  Earnings: $1.23 today | $8.45 week | $32.10 month         │
│                                                             │
│  Memory Usage                    [1H] [6H] [24H] [7D]      │
│  📈 ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁▂▃▄▅▆▇█▇▆▅▄                            │
│                                                             │
│  Activity Timeline              ● OMLX ● Darkbloom ● Idle  │
│  ████████░░░░░░██████████████░░░░░░░░████████████████████  │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Dashboard Features

- **Live status** — Current state, uptime, connection status
- **System metrics** — OMLX models, Darkbloom status, memory usage
- **Earnings card** — Today/week/month/total with estimated hourly rate
- **Memory chart** — Historical memory usage with range selector
- **Activity timeline** — Visual state history (OMLX/Darkbloom/Idle)
- **Earnings chart** — Cumulative and per-session earnings over time
- **Session history** — Recent Darkbloom sessions with earnings
- **Transition log** — State change history with timing

## Configuration

Configuration file: `~/.config/dark-bloom-manager/config.toml`

You can configure via:
1. **CLI flags** (highest priority) - `--omlx-endpoint`, `--omlx-api-key`, etc.
2. **Environment variables** - `OMLX_ENDPOINT`, `OMLX_API_KEY`, `OMLX_PORT`
3. **Config file** (lowest priority) - `~/.config/dark-bloom-manager/config.toml`

### Quick Configuration

```bash
# Interactive wizard (recommended for first-time setup)
dark-bloom-manager config init

# Or set values directly
dark-bloom-manager config set omlx.endpoint http://localhost:8000
dark-bloom-manager config set omlx.api_key your-secret-key
dark-bloom-manager config set omlx.idle_threshold 60
dark-bloom-manager config set darkbloom.model qwen3.5-27b-claude-opus-8bit
dark-bloom-manager config set darkbloom.model_ram 36
```

### Available Config Keys

| Key | Description | Default |
|-----|-------------|---------|
| `omlx.endpoint` | OMLX server URL | `http://localhost:8000` |
| `omlx.api_key` | OMLX API key | (none) |
| `omlx.idle_threshold` | Seconds before switching | `60` |
| `omlx.poll_interval` | Polling interval (seconds) | `5` |
| `omlx.min_idle_polls` | Consecutive idle polls required | `3` |
| `darkbloom.binary` | Path to darkbloom binary | `darkbloom` |
| `darkbloom.model` | Model to serve | `qwen3.5-27b-claude-opus-8bit` |
| `darkbloom.model_ram` | Model RAM requirement (GB) | `36` |
| `darkbloom.shutdown_strategy` | `graceful` or `immediate` | `graceful` |
| `dashboard.enabled` | Enable dashboard | `true` |
| `dashboard.port` | Dashboard port | `9090` |
| `memory.min_available` | Min free RAM (GB) | `40` |

### Full Config File Reference

```toml
[daemon]
log_level = "info"
data_dir = "~/.local/share/dark-bloom-manager"

[dashboard]
enabled = true
port = 9090
bind = "127.0.0.1"

[omlx]
endpoint = "http://localhost:8000"
api_key = "your-omlx-api-key"        # Optional, if OMLX requires auth
poll_interval_secs = 5
idle_threshold_secs = 60              # Seconds of inactivity before switching
min_idle_polls = 3                    # Consecutive idle readings required
request_timeout_secs = 5
unreachable_behavior = "assume_active"  # Safe default

[darkbloom]
binary_path = "darkbloom"
model = "qwen3.5-27b-claude-opus-8bit"
startup_timeout_secs = 60
shutdown_timeout_secs = 120
shutdown_strategy = "graceful"        # Wait for in-flight requests
model_ram_gb = 36                     # RAM required for the model

[memory]
min_available_gb = 40                 # Minimum free RAM before starting Darkbloom
check_interval_secs = 2

[analytics]
enabled = true
snapshot_interval_secs = 60
retention_days = 90
```

## How It Works

### State Machine

```
                OMLX Active
                     │
                     │ Idle for 60s
                     ▼
              Unloading OMLX ──────► Starting Darkbloom
                                            │
                                            ▼
           OMLX Request ◄───────── Darkbloom Active
                │                          │
                │                          │ (earning $$$)
                ▼                          │
         Stopping Darkbloom ◄──────────────┘
                │
                ▼
           OMLX Active
```

### Transition Flow

**OMLX → Darkbloom** (when idle):
1. Detect OMLX idle (no requests for 60s)
2. Unload all OMLX models via admin API
3. Verify RAM is freed
4. Start Darkbloom provider
5. Begin earning from decentralized inference

**Darkbloom → OMLX** (when you need local inference):
1. Detect OMLX activity (request comes in)
2. Wait for any in-flight Darkbloom request
3. Stop Darkbloom gracefully
4. OMLX auto-loads models on demand

## API Endpoints

The dashboard server exposes these endpoints:

| Endpoint | Description |
|----------|-------------|
| `GET /api/status` | Current daemon and system status |
| `GET /api/analytics` | Aggregated statistics |
| `GET /api/memory-history?hours=24` | Memory time-series |
| `GET /api/state-timeline?hours=24` | State changes over time |
| `GET /api/earnings` | Live + summary earnings |
| `GET /api/earnings-history?hours=168` | Earnings time-series |
| `GET /api/sessions` | Recent Darkbloom sessions |
| `GET /api/transitions` | State transition history |
| `GET /health` | Health check |

## Development

```bash
# Run tests
cargo test

# Build debug
cargo build

# Build release
cargo build --release

# Run with debug logging
dark-bloom-manager run --foreground -vv
```

### Project Structure

```
src/
├── main.rs           # CLI entry point
├── lib.rs            # Public types
├── config/           # TOML configuration
├── daemon/           # Main loop, state machine, signals
├── omlx/             # OMLX client and activity monitor
├── darkbloom/        # Darkbloom CLI controller
├── decision/         # State transition logic
├── memory/           # System memory monitoring
├── analytics/        # SQLite storage
├── dashboard/        # Axum web server + UI
└── launchd/          # macOS service management
```

## Troubleshooting

### Darkbloom not starting

```bash
# Check if darkbloom CLI is available
which darkbloom
darkbloom status

# Check daemon logs
tail -f ~/.local/share/dark-bloom-manager/daemon.log
```

### OMLX not detected

```bash
# Verify OMLX is running
curl http://localhost:8000/health

# Check OMLX admin API
curl http://localhost:8000/admin/api/models
```

### Memory issues

```bash
# Check system memory
dark-bloom-manager status --json | jq '.memory'

# Increase idle threshold if transitions are too frequent
dark-bloom-manager config --edit
# Set idle_threshold_secs = 120
```

## License

MIT

## Related Projects

- [Darkbloom (d-inference)](https://github.com/Layr-Labs/d-inference) — Decentralized private inference on Apple Silicon
- [OMLX](https://github.com/jundot/omlx) — LLM inference server with continuous batching for macOS
