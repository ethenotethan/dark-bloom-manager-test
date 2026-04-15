use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

use dark_bloom_manager::{
    config::{Config, ConfigOverrides},
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

    // ===== OMLX Configuration =====
    
    /// OMLX server endpoint URL
    #[arg(long, global = true, env = "OMLX_ENDPOINT")]
    omlx_endpoint: Option<String>,

    /// OMLX server port (shorthand for updating endpoint)
    #[arg(long, global = true, env = "OMLX_PORT")]
    omlx_port: Option<u16>,

    /// OMLX API key for authentication
    #[arg(long, global = true, env = "OMLX_API_KEY")]
    omlx_api_key: Option<String>,

    /// Seconds of OMLX inactivity before switching to Darkbloom
    #[arg(long, global = true)]
    idle_threshold: Option<u64>,

    // ===== Darkbloom Configuration =====
    
    /// Path to darkbloom binary
    #[arg(long, global = true)]
    darkbloom_binary: Option<String>,

    /// Darkbloom model to serve
    #[arg(long, global = true)]
    darkbloom_model: Option<String>,

    /// RAM required for Darkbloom model (in GB)
    #[arg(long, global = true)]
    darkbloom_model_ram: Option<f64>,

    // ===== Dashboard Configuration =====
    
    /// Dashboard server port
    #[arg(long, global = true)]
    dashboard_port: Option<u16>,

    /// Disable dashboard server
    #[arg(long, global = true)]
    no_dashboard: bool,

    // ===== Memory Configuration =====
    
    /// Minimum available memory (GB) before starting Darkbloom
    #[arg(long, global = true)]
    min_memory: Option<f64>,

    #[command(subcommand)]
    command: Commands,
}

impl Cli {
    fn to_overrides(&self) -> ConfigOverrides {
        ConfigOverrides {
            omlx_endpoint: self.omlx_endpoint.clone(),
            omlx_port: self.omlx_port,
            omlx_api_key: self.omlx_api_key.clone(),
            idle_threshold: self.idle_threshold,
            darkbloom_binary: self.darkbloom_binary.clone(),
            darkbloom_model: self.darkbloom_model.clone(),
            darkbloom_model_ram: self.darkbloom_model_ram,
            dashboard_port: self.dashboard_port,
            dashboard_disabled: self.no_dashboard,
            min_available_memory: self.min_memory,
        }
    }
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

    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    
    /// Open config file in editor
    Edit,
    
    /// Validate configuration
    Validate,
    
    /// Set a configuration value
    Set {
        /// Config key (e.g., "omlx.endpoint", "darkbloom.model")
        key: String,
        /// Value to set
        value: String,
    },
    
    /// Get a configuration value
    Get {
        /// Config key to retrieve
        key: String,
    },
    
    /// Interactive setup wizard
    Init {
        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },
    
    /// Interactive config update (auto hot-reloads if daemon is running)
    Update,
    
    /// Show config file path
    Path,
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

    // Load configuration with CLI overrides
    let overrides = cli.to_overrides();
    let config = Config::load_with_overrides(cli.config.as_deref(), &overrides)?;

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

        Commands::Config { action } => {
            handle_config_command(action, cli.config.as_deref()).await?;
        }
    }

    Ok(())
}

async fn handle_config_command(action: Option<ConfigAction>, config_path: Option<&std::path::Path>) -> Result<()> {
    let path = config_path.map(PathBuf::from).unwrap_or_else(Config::default_path);
    
    match action.unwrap_or(ConfigAction::Show) {
        ConfigAction::Show => {
            let config = Config::load(config_path)?;
            println!("{}", toml::to_string_pretty(&config)?);
        }
        
        ConfigAction::Edit => {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
            std::process::Command::new(&editor)
                .arg(&path)
                .status()?;
        }
        
        ConfigAction::Validate => {
            let config = Config::load(config_path)?;
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
        }
        
        ConfigAction::Set { key, value } => {
            let mut config = Config::load(config_path)?;
            config.set_value(&key, &value)?;
            config.save(config_path)?;
            println!("Set {} = {}", key, value);
        }
        
        ConfigAction::Get { key } => {
            let config = Config::load(config_path)?;
            let value = get_config_value(&config, &key)?;
            println!("{}", value);
        }
        
        ConfigAction::Init { force } => {
            run_config_wizard(&path, force).await?;
        }
        
        ConfigAction::Update => {
            run_interactive_config_update(&path).await?;
        }
        
        ConfigAction::Path => {
            println!("{}", path.display());
        }
    }
    
    Ok(())
}

fn get_config_value(config: &Config, key: &str) -> Result<String> {
    let value = match key {
        "omlx.endpoint" => config.omlx.endpoint.clone(),
        "omlx.api_key" => config.omlx.api_key.clone().unwrap_or_default(),
        "omlx.idle_threshold" | "omlx.idle_threshold_secs" => config.omlx.idle_threshold_secs.to_string(),
        "omlx.poll_interval" | "omlx.poll_interval_secs" => config.omlx.poll_interval_secs.to_string(),
        "darkbloom.binary" | "darkbloom.binary_path" => config.darkbloom.binary_path.clone(),
        "darkbloom.model" => config.darkbloom.model.clone(),
        "darkbloom.model_ram" | "darkbloom.model_ram_gb" => config.darkbloom.model_ram_gb.to_string(),
        "dashboard.enabled" => config.dashboard.enabled.to_string(),
        "dashboard.port" => config.dashboard.port.to_string(),
        "memory.min_available" | "memory.min_available_gb" => config.memory.min_available_gb.to_string(),
        _ => anyhow::bail!("Unknown config key: {}", key),
    };
    Ok(value)
}

async fn run_config_wizard(path: &PathBuf, force: bool) -> Result<()> {
    use std::io::{self, Write};
    
    if path.exists() && !force {
        println!("Config file already exists: {}", path.display());
        print!("Overwrite? [y/N] ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }
    
    println!("\n=== Dark Bloom Manager Setup ===\n");
    
    let mut config = Config::default();
    
    // OMLX Configuration
    println!("OMLX Configuration:");
    println!("-------------------");
    
    config.omlx.endpoint = prompt_with_default(
        "OMLX endpoint",
        &config.omlx.endpoint,
    )?;
    
    config.omlx.api_key = prompt_optional("OMLX API key (leave empty if none)")?;
    
    config.omlx.idle_threshold_secs = prompt_number(
        "Idle threshold (seconds before switching to Darkbloom)",
        config.omlx.idle_threshold_secs,
    )?;
    
    println!();
    
    // Darkbloom Configuration
    println!("Darkbloom Configuration:");
    println!("------------------------");
    
    config.darkbloom.binary_path = prompt_with_default(
        "Darkbloom binary path",
        &config.darkbloom.binary_path,
    )?;
    
    config.darkbloom.model = prompt_with_default(
        "Darkbloom model",
        &config.darkbloom.model,
    )?;
    
    config.darkbloom.model_ram_gb = prompt_number(
        "Model RAM requirement (GB)",
        config.darkbloom.model_ram_gb,
    )?;
    
    println!();
    
    // Dashboard Configuration
    println!("Dashboard Configuration:");
    println!("------------------------");
    
    config.dashboard.port = prompt_number(
        "Dashboard port",
        config.dashboard.port,
    )?;
    
    println!();
    
    // Memory Configuration
    println!("Memory Configuration:");
    println!("---------------------");
    
    config.memory.min_available_gb = prompt_number(
        "Minimum available memory (GB) before starting Darkbloom",
        config.memory.min_available_gb,
    )?;
    
    println!();
    
    // Save
    config.save(Some(path))?;
    println!("Configuration saved to: {}", path.display());
    println!();
    println!("You can now start the daemon with:");
    println!("  dark-bloom-manager run --foreground");
    println!();
    println!("Or install as a service:");
    println!("  dark-bloom-manager install && dark-bloom-manager start");
    
    Ok(())
}

fn prompt_with_default(prompt: &str, default: &str) -> Result<String> {
    use std::io::{self, Write};
    
    print!("{} [{}]: ", prompt, default);
    io::stdout().flush()?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    
    Ok(if input.is_empty() {
        default.to_string()
    } else {
        input.to_string()
    })
}

fn prompt_optional(prompt: &str) -> Result<Option<String>> {
    use std::io::{self, Write};
    
    print!("{}: ", prompt);
    io::stdout().flush()?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    
    Ok(if input.is_empty() {
        None
    } else {
        Some(input.to_string())
    })
}

fn prompt_number<T: std::str::FromStr + std::fmt::Display>(prompt: &str, default: T) -> Result<T> 
where
    T::Err: std::fmt::Display,
{
    use std::io::{self, Write};
    
    loop {
        print!("{} [{}]: ", prompt, default);
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();
        
        if input.is_empty() {
            return Ok(default);
        }
        
        match input.parse::<T>() {
            Ok(value) => return Ok(value),
            Err(e) => println!("Invalid input: {}. Please try again.", e),
        }
    }
}

async fn run_interactive_config_update(path: &PathBuf) -> Result<()> {
    use dialoguer::{theme::ColorfulTheme, Confirm, Select};
    
    if !path.exists() {
        println!("No config file found at: {}", path.display());
        println!("Run 'dark-bloom-manager config init' first.");
        return Ok(());
    }
    
    let mut config = Config::load(Some(path))?;
    let theme = ColorfulTheme::default();
    let mut changes_made = false;
    
    println!("\n=== Dark Bloom Manager Configuration ===\n");
    
    // Select category
    let categories = vec![
        "OMLX Settings",
        "Darkbloom Settings", 
        "Memory Settings",
        "Dashboard Settings",
        "Done (Save & Exit)",
    ];
    
    loop {
        let category_idx = Select::with_theme(&theme)
            .with_prompt("Select category to configure")
            .items(&categories)
            .default(0)
            .interact()?;
        
        match category_idx {
            0 => {
                // OMLX Settings
                if update_omlx_settings(&mut config, &theme)? {
                    changes_made = true;
                }
            }
            1 => {
                // Darkbloom Settings
                if update_darkbloom_settings(&mut config, &theme)? {
                    changes_made = true;
                }
            }
            2 => {
                // Memory Settings
                if update_memory_settings(&mut config, &theme)? {
                    changes_made = true;
                }
            }
            3 => {
                // Dashboard Settings
                if update_dashboard_settings(&mut config, &theme)? {
                    changes_made = true;
                }
            }
            4 => {
                // Done - save and exit
                break;
            }
            _ => unreachable!(),
        }
    }
    
    if changes_made {
        // Validate before saving
        if let Err(errors) = config.validate() {
            println!("\nConfiguration has errors:");
            for error in &errors {
                println!("  - {}", error);
            }
            
            if !Confirm::with_theme(&theme)
                .with_prompt("Save anyway?")
                .default(false)
                .interact()?
            {
                println!("Changes discarded.");
                return Ok(());
            }
        }
        
        // Save config
        config.save(Some(path))?;
        println!("\nConfiguration saved to: {}", path.display());
        
        // Auto hot-reload if daemon is running
        match push_config_to_daemon(&config).await {
            Ok(()) => println!("Daemon hot-reloaded."),
            Err(_) => {} // Daemon not running, that's fine
        }
    } else {
        println!("\nNo changes made.");
    }
    
    Ok(())
}

fn update_omlx_settings(config: &mut Config, theme: &dialoguer::theme::ColorfulTheme) -> Result<bool> {
    use dialoguer::{Input, Select};
    
    println!("\n--- OMLX Settings ---\n");
    
    let fields = vec![
        "Endpoint URL",
        "API Key",
        "Idle Threshold (seconds)",
        "Poll Interval (seconds)",
        "Min Idle Polls",
        "Back to main menu",
    ];
    
    let mut changed = false;
    
    loop {
        let field_idx = Select::with_theme(theme)
            .with_prompt("Select field to update")
            .items(&fields)
            .default(0)
            .interact()?;
        
        match field_idx {
            0 => {
                // Endpoint
                let new_value: String = Input::with_theme(theme)
                    .with_prompt("OMLX endpoint URL")
                    .default(config.omlx.endpoint.clone())
                    .interact_text()?;
                if new_value != config.omlx.endpoint {
                    config.omlx.endpoint = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            1 => {
                // API Key
                let current = config.omlx.api_key.clone().unwrap_or_default();
                let display = if current.is_empty() { "(not set)".to_string() } else { "****".to_string() };
                let new_value: String = Input::with_theme(theme)
                    .with_prompt(format!("OMLX API key [current: {}]", display))
                    .allow_empty(true)
                    .default(String::new())
                    .interact_text()?;
                if !new_value.is_empty() {
                    config.omlx.api_key = Some(new_value);
                    changed = true;
                    println!("  Updated!");
                } else if current.is_empty() {
                    println!("  (no change)");
                }
            }
            2 => {
                // Idle threshold
                let new_value: u64 = Input::with_theme(theme)
                    .with_prompt("Idle threshold (seconds before switching to Darkbloom)")
                    .default(config.omlx.idle_threshold_secs)
                    .interact_text()?;
                if new_value != config.omlx.idle_threshold_secs {
                    config.omlx.idle_threshold_secs = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            3 => {
                // Poll interval
                let new_value: u64 = Input::with_theme(theme)
                    .with_prompt("Poll interval (seconds)")
                    .default(config.omlx.poll_interval_secs)
                    .interact_text()?;
                if new_value != config.omlx.poll_interval_secs {
                    config.omlx.poll_interval_secs = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            4 => {
                // Min idle polls
                let new_value: u32 = Input::with_theme(theme)
                    .with_prompt("Min idle polls (consecutive idle checks before switching)")
                    .default(config.omlx.min_idle_polls)
                    .interact_text()?;
                if new_value != config.omlx.min_idle_polls {
                    config.omlx.min_idle_polls = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            5 => break,
            _ => unreachable!(),
        }
    }
    
    Ok(changed)
}

fn update_darkbloom_settings(config: &mut Config, theme: &dialoguer::theme::ColorfulTheme) -> Result<bool> {
    use dialoguer::{Input, Select};
    
    println!("\n--- Darkbloom Settings ---\n");
    
    let fields = vec![
        "Binary Path",
        "Model",
        "Model RAM (GB)",
        "Startup Timeout (seconds)",
        "Shutdown Timeout (seconds)",
        "Shutdown Strategy",
        "Back to main menu",
    ];
    
    let mut changed = false;
    
    loop {
        let field_idx = Select::with_theme(theme)
            .with_prompt("Select field to update")
            .items(&fields)
            .default(0)
            .interact()?;
        
        match field_idx {
            0 => {
                // Binary path
                let new_value: String = Input::with_theme(theme)
                    .with_prompt("Darkbloom binary path")
                    .default(config.darkbloom.binary_path.clone())
                    .interact_text()?;
                if new_value != config.darkbloom.binary_path {
                    config.darkbloom.binary_path = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            1 => {
                // Model
                let new_value: String = Input::with_theme(theme)
                    .with_prompt("Darkbloom model name")
                    .default(config.darkbloom.model.clone())
                    .interact_text()?;
                if new_value != config.darkbloom.model {
                    config.darkbloom.model = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            2 => {
                // Model RAM
                let new_value: f64 = Input::with_theme(theme)
                    .with_prompt("Model RAM requirement (GB)")
                    .default(config.darkbloom.model_ram_gb)
                    .interact_text()?;
                if (new_value - config.darkbloom.model_ram_gb).abs() > 0.01 {
                    config.darkbloom.model_ram_gb = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            3 => {
                // Startup timeout
                let new_value: u64 = Input::with_theme(theme)
                    .with_prompt("Startup timeout (seconds)")
                    .default(config.darkbloom.startup_timeout_secs)
                    .interact_text()?;
                if new_value != config.darkbloom.startup_timeout_secs {
                    config.darkbloom.startup_timeout_secs = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            4 => {
                // Shutdown timeout
                let new_value: u64 = Input::with_theme(theme)
                    .with_prompt("Shutdown timeout (seconds)")
                    .default(config.darkbloom.shutdown_timeout_secs)
                    .interact_text()?;
                if new_value != config.darkbloom.shutdown_timeout_secs {
                    config.darkbloom.shutdown_timeout_secs = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            5 => {
                // Shutdown strategy
                let strategies = vec!["graceful", "immediate"];
                let current_idx = if config.darkbloom.shutdown_strategy == dark_bloom_manager::config::ShutdownStrategy::Graceful { 0 } else { 1 };
                let strategy_idx = Select::with_theme(theme)
                    .with_prompt("Shutdown strategy")
                    .items(&strategies)
                    .default(current_idx)
                    .interact()?;
                let new_strategy = if strategy_idx == 0 {
                    dark_bloom_manager::config::ShutdownStrategy::Graceful
                } else {
                    dark_bloom_manager::config::ShutdownStrategy::Immediate
                };
                if new_strategy != config.darkbloom.shutdown_strategy {
                    config.darkbloom.shutdown_strategy = new_strategy;
                    changed = true;
                    println!("  Updated!");
                }
            }
            6 => break,
            _ => unreachable!(),
        }
    }
    
    Ok(changed)
}

fn update_memory_settings(config: &mut Config, theme: &dialoguer::theme::ColorfulTheme) -> Result<bool> {
    use dialoguer::{Input, Select};
    
    println!("\n--- Memory Settings ---\n");
    
    let fields = vec![
        "Min Available Memory (GB)",
        "Check Interval (seconds)",
        "Back to main menu",
    ];
    
    let mut changed = false;
    
    loop {
        let field_idx = Select::with_theme(theme)
            .with_prompt("Select field to update")
            .items(&fields)
            .default(0)
            .interact()?;
        
        match field_idx {
            0 => {
                // Min available memory
                let new_value: f64 = Input::with_theme(theme)
                    .with_prompt("Minimum available memory (GB) before starting Darkbloom")
                    .default(config.memory.min_available_gb)
                    .interact_text()?;
                if (new_value - config.memory.min_available_gb).abs() > 0.01 {
                    config.memory.min_available_gb = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            1 => {
                // Check interval
                let new_value: u64 = Input::with_theme(theme)
                    .with_prompt("Memory check interval (seconds)")
                    .default(config.memory.check_interval_secs)
                    .interact_text()?;
                if new_value != config.memory.check_interval_secs {
                    config.memory.check_interval_secs = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            2 => break,
            _ => unreachable!(),
        }
    }
    
    Ok(changed)
}

fn update_dashboard_settings(config: &mut Config, theme: &dialoguer::theme::ColorfulTheme) -> Result<bool> {
    use dialoguer::{Confirm, Input, Select};
    
    println!("\n--- Dashboard Settings ---\n");
    
    let fields = vec![
        "Enabled",
        "Port",
        "Bind Address",
        "Back to main menu",
    ];
    
    let mut changed = false;
    
    loop {
        let field_idx = Select::with_theme(theme)
            .with_prompt("Select field to update")
            .items(&fields)
            .default(0)
            .interact()?;
        
        match field_idx {
            0 => {
                // Enabled
                let new_value = Confirm::with_theme(theme)
                    .with_prompt("Enable dashboard?")
                    .default(config.dashboard.enabled)
                    .interact()?;
                if new_value != config.dashboard.enabled {
                    config.dashboard.enabled = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            1 => {
                // Port
                let new_value: u16 = Input::with_theme(theme)
                    .with_prompt("Dashboard port")
                    .default(config.dashboard.port)
                    .interact_text()?;
                if new_value != config.dashboard.port {
                    config.dashboard.port = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            2 => {
                // Bind address
                let new_value: String = Input::with_theme(theme)
                    .with_prompt("Bind address")
                    .default(config.dashboard.bind.clone())
                    .interact_text()?;
                if new_value != config.dashboard.bind {
                    config.dashboard.bind = new_value;
                    changed = true;
                    println!("  Updated!");
                }
            }
            3 => break,
            _ => unreachable!(),
        }
    }
    
    Ok(changed)
}

async fn push_config_to_daemon(config: &Config) -> Result<()> {
    let url = format!(
        "http://{}:{}/api/config",
        config.dashboard.bind,
        config.dashboard.port
    );
    
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(config)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;
    
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("HTTP {}: {}", status, body)
    }
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
