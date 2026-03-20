use clap::{Parser, Subcommand};
use lwid_common::limits::DEFAULT_SERVER;

mod client;
mod config;
mod pull;
mod push;
mod store;

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
        /// Server URL override (for development only)
        #[arg(long, default_value = DEFAULT_SERVER, hide = true)]
        server: String,

        /// Directory to push (default: current directory)
        #[arg(long, default_value = ".")]
        dir: String,

        /// Skip confirmation prompt on first push
        #[arg(short = 'y', long = "yes")]
        yes: bool,

        /// Time-to-live for new projects: 1h, 1d, 7d, 30d, never (default: 7d)
        #[arg(long, default_value = "7d")]
        ttl: String,

        /// Paths to push (default: entire directory)
        paths: Vec<String>,
    },
    /// Pull project files to local directory
    Pull {
        /// Directory to pull into (default: current directory)
        #[arg(long, default_value = ".")]
        dir: String,
    },
    /// Get or set a key-value pair
    Kv {
        /// Store key
        key: String,
        /// Value to set (omit to get)
        value: Option<String>,
        /// Project directory
        #[arg(long, default_value = ".")]
        dir: String,
    },
    /// Get or set a binary blob
    Blob {
        /// Store key
        key: String,
        /// File to upload (use "-" for stdin; omit to download)
        file: Option<String>,
        /// Project directory
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
        Some(Commands::Push {
            server,
            dir,
            yes,
            ttl,
            paths,
        }) => {
            push::run(&dir, &server, yes, &paths, Some(&ttl)).await?;
        }
        Some(Commands::Pull { dir }) => {
            pull::run(&dir).await?;
        }
        Some(Commands::Kv { key, value, dir }) => {
            store::run_kv(&dir, &key, value.as_deref()).await?;
        }
        Some(Commands::Blob { key, file, dir }) => {
            store::run_blob(&dir, &key, file.as_deref()).await?;
        }
        Some(Commands::Info) => {
            let cfg = config::load(".")?;
            let read_key_b64 = base64_url_encode(&cfg.read_key);
            let write_key_b64 = base64_url_encode(&cfg.write_key);
            println!("Project:  {}", cfg.project_id);
            println!("Server:   {DEFAULT_SERVER}");
            println!(
                "Edit URL: {DEFAULT_SERVER}/p/{}#{}:{}",
                cfg.project_id, read_key_b64, write_key_b64
            );
            println!(
                "View URL: {DEFAULT_SERVER}/p/{}#{}",
                cfg.project_id, read_key_b64
            );
        }
        None => {
            // Default: push current dir
            push::run(".", DEFAULT_SERVER, false, &[], Some("7d")).await?;
        }
    }

    Ok(())
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::prelude::*;
    BASE64_URL_SAFE_NO_PAD.encode(bytes)
}
