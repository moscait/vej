//! Remote access: who may call vej from other machines, and how.

pub mod auth;
pub mod tokens;

use anyhow::{Result, bail};

use crate::cli::{RemoteArgs, RemoteCommands, TokenArgs, TokenCommands};
use crate::config::Config;
pub use auth::RemoteAuth;

/// `vej remote token create|list|revoke`.
pub fn handle(args: RemoteArgs, config: &Config) -> Result<()> {
    let RemoteCommands::Token(TokenArgs { command }) = args.command;
    let path = config.remote.tokens_path();
    match command {
        TokenCommands::Create { name } => {
            let token = tokens::create(&path, &name)?;
            println!(
                "✓ Token for '{name}' (shown only once; keep it in the client's secret store):"
            );
            println!();
            println!("    {token}");
            println!();
            println!("Clients send it as:  Authorization: Bearer <token>");
            println!("Only its SHA-256 digest is stored, in {}", path.display());
        }
        TokenCommands::List => {
            let names = tokens::list(&path)?;
            if names.is_empty() {
                println!(
                    "No remote tokens yet ({}). Create one with `vej remote token create <name>`.",
                    path.display()
                );
            }
            for name in names {
                println!("{name}");
            }
        }
        TokenCommands::Revoke { name } => {
            if !tokens::revoke(&path, &name)? {
                bail!("no token named '{name}' in {}", path.display());
            }
            println!("✓ Revoked '{name}' (effective immediately; no restart needed)");
        }
    }
    Ok(())
}
