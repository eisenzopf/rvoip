use clap::{Parser, Subcommand};

mod api_client;
mod commands;
mod config;
mod output;

#[derive(Parser)]
#[command(name = "rvoip", version, about = "rvoip Call Center CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Server URL (overrides config)
    #[arg(long, env = "RVOIP_URL", global = true)]
    url: Option<String>,

    /// Auth token (overrides config)
    #[arg(long, env = "RVOIP_TOKEN", global = true)]
    token: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with the server
    Login,
    /// Show system status dashboard
    Status,
    /// Manage agents
    #[command(subcommand)]
    Agent(AgentCmd),
    /// Manage calls
    #[command(subcommand)]
    Call(CallCmd),
    /// Manage queues
    #[command(subcommand)]
    Queue(QueueCmd),
    /// Manage users
    #[command(subcommand)]
    User(UserCmd),
    /// Manage departments
    #[command(subcommand)]
    Dept(DeptCmd),
    /// Generate reports
    #[command(subcommand)]
    Report(ReportCmd),
    /// Manage CLI configuration
    #[command(subcommand)]
    Config(ConfigCmd),
}

#[derive(Subcommand, Clone)]
pub enum AgentCmd {
    /// List agents
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        dept: Option<String>,
    },
    /// Create a new agent
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        dept: Option<String>,
    },
    /// Delete an agent
    Delete { id: String },
    /// Set agent status
    Status {
        id: String,
        #[arg(long = "set")]
        new_status: String,
    },
}

#[derive(Subcommand, Clone)]
pub enum CallCmd {
    /// List active calls
    List {
        #[arg(long)]
        status: Option<String>,
    },
    /// Show call history
    History {
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Hang up a call
    Hangup { id: String },
    /// Show call statistics
    Stats,
}

#[derive(Subcommand, Clone)]
pub enum QueueCmd {
    /// List queues
    List,
    /// Show queue status
    Status { id: String },
    /// Create a new queue
    Create {
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand, Clone)]
pub enum UserCmd {
    /// List users
    List,
    /// Create a new user
    Create {
        #[arg(long)]
        username: String,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        email: Option<String>,
    },
    /// Delete a user
    Delete { id: String },
    /// List available roles
    Roles,
}

#[derive(Subcommand, Clone)]
pub enum DeptCmd {
    /// List departments
    List,
    /// Create a department
    Create {
        #[arg(long)]
        name: String,
    },
    /// Delete a department
    Delete { id: String },
}

#[derive(Subcommand, Clone)]
pub enum ReportCmd {
    /// Daily report
    Daily {
        #[arg(long)]
        date: Option<String>,
    },
    /// Agent performance report
    Agent { id: String },
    /// Summary report
    Summary,
    /// Export report
    Export {
        #[arg(long)]
        format: Option<String>,
    },
}

#[derive(Subcommand, Clone)]
pub enum ConfigCmd {
    /// Show current configuration
    Show,
    /// Set a configuration value
    Set { key: String, value: String },
    /// Export configuration as JSON
    Export,
    /// Import configuration from file
    Import { file: String },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let mut cfg = config::load_config();

    let base_url = cli
        .url
        .as_deref()
        .unwrap_or(&cfg.server.url)
        .to_string();
    let token = cli
        .token
        .as_deref()
        .or(cfg.auth.token.as_deref())
        .unwrap_or_default()
        .to_string();

    let api = api_client::ApiClient::new(&base_url, &token);

    let result = match cli.command {
        Commands::Login => commands::login::execute(&api, &mut cfg).await,
        Commands::Status => commands::status::execute(&api).await,
        Commands::Agent(cmd) => commands::agent::execute(&api, cmd).await,
        Commands::Call(cmd) => commands::call::execute(&api, cmd).await,
        Commands::Queue(cmd) => commands::queue::execute(&api, cmd).await,
        Commands::User(cmd) => commands::user::execute(&api, cmd).await,
        Commands::Dept(cmd) => commands::department::execute(&api, cmd).await,
        Commands::Report(cmd) => commands::report::execute(&api, cmd).await,
        Commands::Config(cmd) => commands::config_cmd::execute(cmd).map_err(Into::into),
    };

    if let Err(e) = result {
        output::print_error(&format!("{:#}", e));
        std::process::exit(1);
    }
}
