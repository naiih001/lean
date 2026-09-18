use clap::Parser;
use lean::{models, tui};

#[derive(Parser, Debug)]
#[command(name = "lean", about = "lean — light coding assistant (Rust port)")]
struct Args {
    #[arg(
        long,
        help = "Model alias from ~/.lean/models.json (default alias from file)"
    )]
    model: Option<String>,

    #[arg(long, help = "Continue the most recent session")]
    r#continue: bool,

    #[arg(long, help = "Resume session by id (prefix match)")]
    resume: Option<String>,

    #[arg(long, help = "Disable session persistence")]
    no_session: bool,

    #[arg(long, help = "Disable bash guard")]
    bash_guard_disabled: bool,

    #[arg(long, help = "Disable directory guard (CWD confinement)")]
    dir_guard_disabled: bool,

    #[arg(
        long,
        help = "Start in mode (norm, plan, ask, or a custom modes.json mode)"
    )]
    mode: Option<String>,

    #[arg(long, help = "Auto-approve all approval prompts (AUTO mode)")]
    auto_accept: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    // Resolve model alias via ~/.lean/models.json (creates template if missing)
    let resolved = models::resolve(args.model.as_deref())?;
    let model_alias = resolved.alias.clone();
    tui::run(tui::RunOpts {
        model: model_alias,
        continue_session: args.r#continue,
        resume_id: args.resume,
        no_session: args.no_session,
        bash_guard_disabled: args.bash_guard_disabled,
        dir_guard_disabled: args.dir_guard_disabled,
        mode: args.mode,
        auto_accept: args.auto_accept,
    })
    .await
}
