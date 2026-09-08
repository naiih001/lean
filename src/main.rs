mod agent;
mod llm;
mod memory;
mod skills;
mod theme;
mod tools;
mod tui;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "lean", about = "lean — light coding assistant (Rust port)")]
struct Args {
    #[arg(long, default_value = "mimo-v2.5-free")]
    model: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    tui::run(args.model).await
}
