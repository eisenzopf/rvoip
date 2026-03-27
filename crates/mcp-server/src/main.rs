use clap::Parser;

mod api_client;
mod prompts;
mod resources;
mod server;
mod tools;

/// MCP Server for rvoip call center — lets AI models manage the platform.
#[derive(Parser)]
#[command(name = "rvoip-mcp-server", about = "MCP Server for rvoip call center")]
struct Args {
    /// Base URL for the rvoip web-console API
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    base_url: String,

    /// Bearer token for API authentication
    #[arg(long, env = "RVOIP_MCP_TOKEN")]
    token: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    tracing::info!("Starting rvoip MCP server, API base: {}", args.base_url);

    let api = api_client::RvoipApiClient::new(&args.base_url, &args.token);
    server::run_stdio_server(api).await
}
