//! xenify-chat-server serve | invite | user add | user disable | user enable

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xenify-chat-server", about = "The Xenify Chat server")]
struct Cli {
    /// PostgreSQL address. Also read from XENIFY_CHAT_DATABASE.
    #[arg(long, env = "XENIFY_CHAT_DATABASE", default_value = "postgres://xenify@localhost/xenify_chat")]
    database: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (put TLS in front, e.g. Caddy).
    Serve {
        #[arg(long, default_value = "127.0.0.1:7700")]
        listen: String,
        /// Let anyone create an account (default: invite codes only).
        #[arg(long)]
        open_registration: bool,
        /// Let app webhooks point at this computer or a private network
        /// (for testing; never on a server others use).
        #[arg(long)]
        allow_local_webhooks: bool,
    },
    /// Print a new invite code.
    Invite {
        #[arg(long, default_value_t = 7)]
        days: i32,
    },
    /// Accounts.
    User {
        #[command(subcommand)]
        action: UserAction,
    },
}

#[derive(Subcommand)]
enum UserAction {
    /// Create an account (the password is read from XENIFY_CHAT_PASSWORD).
    Add {
        name: String,
        /// The name others see ("Alex Lane").
        #[arg(long, default_value = "")]
        display_name: String,
    },
    /// Stop an account signing in (its sessions stop working).
    Disable {
        name: String,
    },
    Enable {
        name: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Serve { listen, open_registration, allow_local_webhooks } => {
            let server = xenify_chat_server::Server::start(xenify_chat_server::Config {
                database: cli.database,
                open_registration,
                local_webhooks: allow_local_webhooks,
            })
            .await?;
            let l = tokio::net::TcpListener::bind(&listen).await.map_err(|e| format!("{listen}: {e}"))?;
            println!("Xenify Chat server on {listen}");
            axum::serve(l, server.router())
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .map_err(|e| e.to_string())
        }
        other => {
            let pool = xenify_chat_server::pool(&cli.database)?;
            xenify_chat_server::migrate(&pool).await?;
            let c = pool.get().await.map_err(|e| e.to_string())?;
            match other {
                Command::Invite { days } => println!("{}", xenify_chat_server::invite(&pool, days).await?),
                Command::User { action: UserAction::Add { name, display_name } } => {
                    let pw = std::env::var("XENIFY_CHAT_PASSWORD")
                        .map_err(|_| "set XENIFY_CHAT_PASSWORD to the new password".to_string())?;
                    let name = name.to_lowercase();
                    if !xenify_chat_server::valid_username(&name) {
                        return Err("user names are 2–32 letters, digits, dots, dashes or underscores".into());
                    }
                    xenify_chat_server::db::create_user(
                        &c,
                        &name,
                        &display_name,
                        &xenify_chat_server::hash_password(&pw),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    println!("Added {name}.");
                }
                Command::User { action: UserAction::Disable { name } } => {
                    c.execute("UPDATE users SET disabled = TRUE WHERE username = $1", &[&name.to_lowercase()])
                        .await
                        .map_err(|e| e.to_string())?;
                    c.execute(
                        "DELETE FROM sessions WHERE user_id = (SELECT id FROM users WHERE username = $1)",
                        &[&name.to_lowercase()],
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    println!("Disabled {name}.");
                }
                Command::User { action: UserAction::Enable { name } } => {
                    c.execute("UPDATE users SET disabled = FALSE WHERE username = $1", &[&name.to_lowercase()])
                        .await
                        .map_err(|e| e.to_string())?;
                    println!("Enabled {name}.");
                }
                Command::Serve { .. } => unreachable!(),
            }
            Ok(())
        }
    }
}
