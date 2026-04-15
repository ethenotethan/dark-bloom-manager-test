use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

use dark_bloom_manager::{
    config::Config,
    daemon::Daemon,
    launchd,
};

#[derive(Parser)]
#[command(name = "dark-bloom-manager")]
#[command(author, version, about = "Supervisor daemon for Darkbloom provider with OMLX coordination")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Suppress output
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the daemon
    Run {
        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },

    /// Install as launchd service
    Install,

    /// Uninstall launchd service
    Uninstall,

    /// Start the launchd service
    Start,

    /// Stop the launchd service
    Stop,

    /// Show current status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Open dashboard in browser
    Dashboard,

    /// Show analytics summary
    Analytics {
        /// Time period: hour, day, week, month
        #[arg(long, default_value = "day")]
        period: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show or edit configuration
    Config {
        /// Open config in editor
        #[arg(long)]
        edit: bool,

        /// Validate configuration
        #[arg(long)]
        validate: bool,
    },
}

fn setup_logging(verbose: u8, quiet: bool) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    setup_logging(cli.verbose, cli.quiet);

    // Load configuration
    let config = Config::load(cli.config.as_deref())?;

    match cli.command {
        Commands::Run { foreground } => {
            info!("Starting dark-bloom-manager daemon");
            let daemon = Daemon::new(config).await?;
            daemon.run(foreground).await?;
        }

        Commands::Install => {
            info!("Installing launchd service");
            launchd::install(&config)?;
            println!("Installed launchd service: ai.darkbloom.manager");
            println!("Start with: dark-bloom-manager start");
        }

        Commands::Uninstall => {
            info!("Uninstalling launchd service");
            launchd::uninstall()?;
            println!("Uninstalled launchd service");
        }

        Commands::Start => {
            launchd::start()?;
            println!("Started dark-bloom-manager service");
        }

        Commands::Stop => {
            launchd::stop()?;
            println!("Stopped dark-bloom-manager service");
        }

        Commands::Status { json } => {
            let status = dark_bloom_manager::get_status(&config).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_status(&status);
            }
        }

        Commands::Dashboard => {
            let url = format!("http://localhost:{}/dashboard", config.dashboard.port);
            println!("Opening dashboard: {}", url);
            open::that(&url)?;
        }

        Commands::Analytics { period, json } => {
            let analytics = dark_bloom_manager::get_analytics(&config, &period).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&analytics)?);
            } else {
                print_analytics(&analytics);
            }
        }

        Commands::Config { edit, validate } => {
            if validate {
                match config.validate() {
                    Ok(()) => println!("Configuration is valid"),
                    Err(errors) => {
                        eprintln!("Configuration errors:");
                        for error in errors {
                            eprintln!("  - {}", error);
                        }
                        std::process::exit(1);
                    }
                }
            } else if edit {
                let config_path = Config::default_path();
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
                std::process::Command::new(&editor)
                    .arg(&config_path)
                    .status()?;
            } else {
                println!("Config path: {}", Config::default_path().display());
                println!("{}", toml::to_string_pretty(&config)?);
            }
        }
    }

    Ok(())
}

fn print_status(status: &dark_bloom_manager::Status) {
    println!("Dark Bloom Manager Status");
    println!("========================");
    println!();
    
    println!("Daemon:");
    println!("  Running: {}", status.daemon.running);
    if let Some(uptime) = status.daemon.uptime_secs {
        println!("  Uptime: {}s", uptime);
    }
    if let Some(pid) = status.daemon.pid {
        println!("  PID: {}", pid);
    }
    println!();

    println!("State: {:?}", status.state);
    println!();

    println!("OMLX:");
    println!("  Reachable: {}", status.omlx.reachable);
    println!("  Loaded models: {}", status.omlx.loaded_models.len());
    for model in &status.omlx.loaded_models {
        println!("    - {}", model);
    }
    println!("  Memory: {:.1} GB", status.omlx.memory_gb);
    if let Some(idle) = status.omlx.idle_duration_secs {
        println!("  Idle: {}s", idle);
    }
    println!();

    println!("Darkbloom:");
    println!("  Running: {}", status.darkbloom.running);
    if status.darkbloom.running {
        println!("  Connected: {}", status.darkbloom.connected);
        if let Some(model) = &status.darkbloom.model {
            println!("  Model: {}", model);
        }
        if let Some(uptime) = status.darkbloom.uptime_secs {
            println!("  Uptime: {}s", uptime);
        }
    }
    println!();

    println!("Memory:");
    println!("  System total: {:.1} GB", status.memory.system_total_gb);
    println!("  Available: {:.1} GB", status.memory.system_available_gb);
}

fn print_analytics(analytics: &dark_bloom_manager::AnalyticsSummary) {
    println!("Analytics Summary ({:?})", analytics.period);
    println!("===================");
    println!();
    
    println!("Time Allocation:");
    println!("  OMLX active: {:.1}%", analytics.omlx_active_pct);
    println!("  Darkbloom active: {:.1}%", analytics.darkbloom_active_pct);
    println!("  Idle: {:.1}%", analytics.idle_pct);
    println!();

    println!("OMLX:");
    println!("  Requests: {}", analytics.omlx_requests);
    println!();

    println!("Darkbloom:");
    println!("  Requests served: {}", analytics.darkbloom_requests_served);
    println!("  Earnings: ${:.4}", analytics.darkbloom_earnings_usd);
    println!();

    println!("Transitions: {}", analytics.transitions_count);
    if analytics.transitions_count > 0 {
        println!("  Avg duration: {}ms", analytics.avg_transition_duration_ms);
    }
}
