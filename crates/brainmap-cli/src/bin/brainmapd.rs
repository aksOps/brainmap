use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "brainmapd", about = "Optional Brainmap local daemon")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Start,
    Stop,
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Start => {
            println!("brainmapd optional daemon not running; CLI fallback is active");
        }
        Command::Stop => {
            println!("brainmapd optional daemon not running");
        }
        Command::Status => {
            println!("status: stopped\nfallback: direct brainmap CLI");
        }
    }
    Ok(())
}
