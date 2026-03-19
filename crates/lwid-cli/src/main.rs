use clap::{Parser, Subcommand};

mod client;
mod config;
mod pull;
mod push;

#[derive(Parser)]
#[command(name = "lwid", version, about = "Push and pull encrypted projects to lookwhatidid")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Push local files to lookwhatidid (creates project if needed)
    Push {
        /// Server URL (overrides .lwid.json)
        #[arg(long, default_value = "https://lookwhatidid.ovh")]
        server: String,

        /// Directory to push (default: current directory)
        #[arg(long, default_value = ".")]
        dir: String,
    },
    /// Pull project files to local directory
    Pull {
        /// Directory to pull into (default: current directory)
        #[arg(long, default_value = ".")]
        dir: String,
    },
    /// Show project info
    Info,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Push { server, dir }) => {
            push::run(&dir, &server).await?;
        }
        Some(Commands::Pull { dir }) => {
            pull::run(&dir).await?;
        }
        Some(Commands::Info) => {
            let cfg = config::load(".")?;
            let read_key_b64 = base64_url_encode(&cfg.read_key);
            let write_key_b64 = base64_url_encode(&cfg.write_key);
            println!("Project:  {}", cfg.project_id);
            println!("Server:   {}", cfg.server);
            println!(
                "Edit URL: {}/p/{}#{}:{}",
                cfg.server, cfg.project_id, read_key_b64, write_key_b64
            );
            println!(
                "View URL: {}/p/{}#{}",
                cfg.server, cfg.project_id, read_key_b64
            );
        }
        None => {
            // Default: push
            push::run(".", "https://lookwhatidid.ovh").await?;
        }
    }

    Ok(())
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::prelude::*;
    BASE64_URL_SAFE_NO_PAD.encode(bytes)
}
