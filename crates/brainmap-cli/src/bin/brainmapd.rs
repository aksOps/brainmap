use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "brainmapd", about = "Optional Brainmap local daemon")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(name = "build-info")]
    BuildInfo,
    Start {
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8777)]
        port: u16,
    },
    Stop,
    Status {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8777)]
        port: u16,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::BuildInfo => brainmap_cli::build_info::print_build_info()?,
        Command::Start { vault, host, port } => {
            brainmap_cli::web::serve(vault, &host, port, false)?
        }
        Command::Stop => {
            println!("brainmapd stop: terminate the foreground process running brainmapd start");
        }
        Command::Status { host, port } => status(&host, port),
    }
    Ok(())
}

fn status(host: &str, port: u16) {
    let addr = format!("{host}:{port}");
    let running = addr
        .parse::<SocketAddr>()
        .ok()
        .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(200)).ok())
        .is_some();
    if running {
        println!("status: running\nui: http://{addr}");
    } else {
        println!("status: stopped\nui: http://{addr}");
    }
}
