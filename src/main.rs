use anyhow::Result;
use clap::Parser;
use snapit::cli::{run, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}
