use anyhow::Result;
use clap::Parser;
use octo_core::{Config, Hub};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long, default_value = "127.0.0.1:7878")]
    addr: String,
    /// Built web UI to serve. Defaults to ./web/dist, then the repo's web/dist;
    /// skipped if the directory does not exist.
    #[arg(long)]
    web_dir: Option<PathBuf>,
    /// Require `Authorization: Bearer <token>` on /api/*. Open when omitted.
    #[arg(long)]
    token: Option<String>,
    /// Read the token from this file; a random one (0600) is created on first start.
    #[arg(long, conflicts_with = "token")]
    token_file: Option<PathBuf>,
    /// Probe http://<ADDR>/healthz and exit 0/1 (for container HEALTHCHECK).
    #[arg(long, value_name = "ADDR")]
    healthcheck: Option<String>,
    #[arg(long)]
    claude_bin: Option<PathBuf>,
    #[arg(long)]
    agy_bin: Option<PathBuf>,
    /// Claude Code projects dir (default: ~/.claude/projects).
    #[arg(long)]
    claude_projects_dir: Option<PathBuf>,
    /// Antigravity CLI dir (default: ~/.gemini/antigravity-cli).
    #[arg(long)]
    agy_dir: Option<PathBuf>,
    /// Run a relay on a SEPARATE listener: only /relay (WebSocket) and /healthz.
    /// The API listener never serves /relay.
    #[arg(long)]
    relay_addr: Option<String>,
    /// Run only the relay (needs --relay-addr); no API listener, hub or node.
    #[arg(long, requires = "relay_addr")]
    relay_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = Args::parse();
    if let Some(a) = &args.healthcheck {
        if let Err(e) = octo_server::healthcheck(a) {
            eprintln!("unhealthy: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(p) = &args.token_file {
        args.token = Some(octo_server::load_or_create_token(p)?);
    }
    let relay_listener = match &args.relay_addr {
        Some(a) => {
            let l = tokio::net::TcpListener::bind(a).await?;
            println!("Octo relay: ws://{}/relay", l.local_addr()?);
            let relay = octo_net::relay::Relay::new(Default::default())?;
            Some((l, octo_server::relay_app(relay)))
        }
        None => None,
    };
    if args.relay_only {
        let Some((l, app)) = relay_listener else {
            anyhow::bail!("--relay-only needs --relay-addr");
        };
        axum::serve(l, app).await?;
        return Ok(());
    }
    let api_addr: std::net::SocketAddr = args.addr.parse()?;
    octo_server::check_exposure(api_addr, args.token.as_deref()).map_err(anyhow::Error::msg)?;
    let mut config = Config::new(args.data_dir);
    if let Some(p) = args.claude_bin {
        config.claude_bin = p;
    }
    if let Some(p) = args.agy_bin {
        config.agy_bin = p;
    }
    if let Some(p) = args.claude_projects_dir {
        config.claude_projects_dir = p;
    }
    if let Some(p) = args.agy_dir {
        config.agy_dir = p;
    }
    let web_dir = args
        .web_dir
        .into_iter()
        .chain([
            PathBuf::from("web/dist"),
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/dist")),
        ])
        .find(|p| p.is_dir());
    let web_built = web_dir.is_some();
    let web_dir = web_dir.map(octo_server::WebAssets::Dir);
    let data_dir = config.data_dir.clone();
    let hub = Arc::new(Hub::new(config, &octo_server::node::node_id(&data_dir)?)?);
    let node = octo_server::node::Node::start(hub.clone(), &data_dir, Default::default())?;
    let app = octo_server::app_with_node(hub, node, web_dir, args.token);
    if let Some((l, relay_app)) = relay_listener {
        tokio::spawn(async move {
            if let Err(e) = axum::serve(l, relay_app).await {
                eprintln!("relay listener: {e}");
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(api_addr).await?;
    println!("Octo: http://{}", listener.local_addr()?);
    if !web_built {
        println!("web UI not built: cd web && npm ci && npm run build");
    }
    axum::serve(listener, app).await?;
    Ok(())
}
