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
    /// Built web UI to serve. Defaults to ./web/dist when it exists; an explicit path that is
    /// not a directory is an error.
    #[arg(long)]
    web_dir: Option<PathBuf>,
    /// Require `Authorization: Bearer <token>` on /api/*. Open when omitted. Visible in the
    /// process list: prefer --token-file.
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
    /// Private relay: admit only nodes whose relay URL carries `?key=<secret>`. The secret is
    /// read from this file; a random one (0600) is created on first start.
    #[arg(long, requires = "relay_addr")]
    relay_secret_file: Option<PathBuf>,
    /// Address devices use to reach this relay (e.g. `wss://relay.example.com/relay` behind a
    /// proxy); only printed in the relay code. Default: the local listener.
    #[arg(long, requires = "relay_addr")]
    relay_public_url: Option<String>,
}

/// Resolves on Ctrl-C or SIGTERM.
async fn termination() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// A future that resolves once `stop` flips to true (or its sender is gone).
async fn stopped(mut stop: tokio::sync::watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// Where a local client reaches a listener: a wildcard bind is dialed on loopback.
fn callback_addr(mut a: std::net::SocketAddr) -> std::net::SocketAddr {
    if a.ip().is_unspecified() {
        a.set_ip(match a {
            std::net::SocketAddr::V4(_) => std::net::Ipv4Addr::LOCALHOST.into(),
            std::net::SocketAddr::V6(_) => std::net::Ipv6Addr::LOCALHOST.into(),
        });
    }
    a
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
    let relay_secret = match &args.relay_secret_file {
        Some(p) => Some(octo_server::load_or_create_token(p)?),
        None => None,
    };
    if let Some(t) = &args.token {
        octo_server::check_token(t).map_err(anyhow::Error::msg)?;
    }
    // Everything that can be wrong with the configuration is checked before any port is bound.
    if args.relay_only && args.relay_addr.is_none() {
        anyhow::bail!("--relay-only needs --relay-addr");
    }
    if let Some(u) = &args.relay_public_url
        && (!(u.starts_with("ws://") || u.starts_with("wss://")) || u.contains('?'))
    {
        anyhow::bail!("--relay-public-url must be a ws:// or wss:// URL without a query");
    }
    let relay_sock: Option<std::net::SocketAddr> =
        args.relay_addr.as_deref().map(str::parse).transpose()?;
    let api_addr: std::net::SocketAddr = args.addr.parse()?;
    if !args.relay_only {
        octo_server::check_exposure(api_addr, args.token.as_deref()).map_err(anyhow::Error::msg)?;
    }
    let web_dir = match args.web_dir {
        Some(p) if p.is_dir() => Some(p),
        Some(p) => anyhow::bail!("--web-dir {} is not a directory", p.display()),
        None => Some(PathBuf::from("web/dist")).filter(|p| p.is_dir()),
    };
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        termination().await;
        let _ = stop_tx.send(true);
    });
    let relay_task = match relay_sock {
        Some(sock) => {
            let l = tokio::net::TcpListener::bind(sock).await?;
            let local = format!("ws://{}/relay", l.local_addr()?);
            println!("Octo relay: {local}");
            let secret = relay_secret.clone();
            if let Some(key) = &secret {
                println!("Octo relay: private (clients need ?key=<secret> in the relay URL)");
                // What the first device pastes into Settings, Network, Create a network.
                let public = args.relay_public_url.as_deref().unwrap_or(&local);
                println!("Octo relay code: {public}?key={key}");
            }
            let relay = octo_net::relay::Relay::with_secret(Default::default(), secret)?;
            let app = octo_server::relay_app(relay);
            let stop = stop_rx.clone();
            Some(tokio::spawn(async move {
                if let Err(e) = octo_server::serve(l, app, stopped(stop)).await {
                    eprintln!("octo: relay listener failed: {e}");
                }
            }))
        }
        None => None,
    };
    if args.relay_only {
        if let Some(t) = relay_task {
            t.await?;
        }
        return Ok(());
    }
    // Bind first: Claude's approval prompts call back to the address actually bound.
    let listener = tokio::net::TcpListener::bind(api_addr).await?;
    let mut config = Config::new(args.data_dir);
    config.local_api = Some(callback_addr(listener.local_addr()?));
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
    let web_built = web_dir.is_some();
    let web = web_dir.map(octo_server::WebAssets::Dir);
    let data_dir = config.data_dir.clone();
    let hub = Arc::new(Hub::new(config, &octo_server::node::node_id(&data_dir)?)?);
    let node = octo_server::node::Node::start(hub.clone(), &data_dir, Default::default())?;
    let allowed_host = Some(match api_addr.ip() {
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
        ip => ip.to_string(),
    });
    let app = octo_server::app(
        hub.clone(),
        octo_server::AppConfig {
            web,
            token: args.token,
            node: Some(node),
            allowed_host,
        },
    );
    println!("Octo: http://{}", listener.local_addr()?);
    if !web_built {
        println!("web UI not built: cd web && npm ci && npm run build");
    }
    let served = octo_server::serve(listener, app, stopped(stop_rx)).await;
    // Stop accepting happened above; now end agent processes, then the relay listener.
    hub.shutdown();
    if let Some(t) = relay_task {
        let _ = t.await;
    }
    served?;
    Ok(())
}
