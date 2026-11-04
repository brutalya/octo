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
    #[arg(long)]
    claude_bin: Option<PathBuf>,
    #[arg(long)]
    agy_bin: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut config = Config::new(args.data_dir);
    if let Some(p) = args.claude_bin {
        config.claude_bin = p;
    }
    if let Some(p) = args.agy_bin {
        config.agy_bin = p;
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
    let app = octo_server::app(Arc::new(Hub::new(config)?), web_dir);
    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    println!("Octo: http://{}", listener.local_addr()?);
    if !web_built {
        println!("web UI not built: cd web && npm ci && npm run build");
    }
    axum::serve(listener, app).await?;
    Ok(())
}
