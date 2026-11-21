//! Entry point. One binary carries both the server and the operational tools.
//! The client (`put`/`get`/`delete`) is the separate TypeScript CLI (ADR-009).

mod commands;

use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Parser, Subcommand};

use capsulebin_server::config::Config;
use capsulebin_server::healthcheck::probe_address;

#[derive(Parser)]
#[command(name = "capsulebin", about = "CapsuleBin - Paste it. Seal it. Share it.", version)]
struct Cli {
    /// Configuration file. Without it, defaults plus `CAPSULEBIN__` environment
    /// variables apply.
    #[arg(long, short = 'c', global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the server.
    Serve {
        /// Address to listen on. Overrides `server.bind`.
        #[arg(long)]
        bind: Option<String>,
    },
    /// Verify configuration, database and migrations without listening.
    Check,
    /// Ask the running server whether it is healthy; exit 0 if it is.
    ///
    /// Meant for container health checks, where no `curl` is available. Probes
    /// `/health/ready` on the address from `server.bind` (`0.0.0.0` and `[::]`
    /// become loopback). Needs only that address: it validates nothing else in
    /// the configuration, writes nothing and opens neither the database nor a
    /// port.
    Healthcheck {
        /// Probe `/health/live` (the process answers) instead of `/health/ready`
        /// (assets, database and schema are usable). Use this where an unhealthy
        /// container gets restarted: a database that is briefly unavailable is a
        /// reason to wait, not to restart.
        #[arg(long)]
        live: bool,
        /// Address to probe instead of the one derived from `server.bind`, for a
        /// server started with `serve --bind`. No configuration is read then.
        #[arg(long)]
        address: Option<String>,
    },
    /// Apply pending migrations and exit.
    Migrate,
    /// Apply the retention policy once and exit.
    ///
    /// Runs the same rules as the scheduled job, so it doubles as the way to force
    /// a purge after changing the thresholds.
    Prune {
        /// Rows removed per rule per statement. Defaults to `purge.batch_size`.
        #[arg(long)]
        batch_size: Option<i64>,
    },
    /// Generate a creation token and the hash to configure for it.
    ///
    /// The hash is domain-separated, so it cannot be reproduced with a plain
    /// `sha256sum` of the token. This subcommand is the supported way to produce
    /// a `creation.token_hashes` entry.
    Token,
    /// Write a consistent snapshot of the database to a new file.
    ///
    /// Safe while the server is running. The file is compacted (pages freed by
    /// deleted pastes are not copied) and readable by the owner only.
    Backup {
        /// Target file. Must not exist.
        #[arg(long, short = 'o')]
        output: PathBuf,
    },
    /// Replace the configured database with a backup. Stop the server first.
    ///
    /// The backup is verified before anything is touched; only its main file is
    /// used, never a -wal next to it. Restore only backups you made yourself.
    Restore {
        /// Backup file to restore.
        #[arg(long, short = 'i')]
        input: PathBuf,
        /// Replace an existing database. It is kept, together with its -wal and
        /// -shm, as `<file>.pre-restore-<timestamp>`, never deleted.
        #[arg(long)]
        force: bool,
        /// Verify the backup and test-migrate a private copy of it; change
        /// nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Abuse reports submitted through the report page.
    #[command(subcommand)]
    Reports(ReportsCommand),
    /// Disable, delete or restore a paste by id - without decrypting anything.
    #[command(subcommand)]
    Moderation(ModerationCommand),
}

#[derive(Subcommand)]
enum ReportsCommand {
    /// List reports, oldest first.
    List {
        /// open, reviewing, closed or all.
        #[arg(long, default_value = "open")]
        status: String,
        #[arg(long, default_value_t = 100)]
        limit: i64,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Show one report in full.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Mark an open report as being reviewed.
    Review { id: String },
    /// Close a report.
    Close {
        id: String,
        /// action_taken, no_action, duplicate or invalid.
        #[arg(long)]
        resolution: String,
    },
}

#[derive(Subcommand)]
enum ModerationCommand {
    /// Block a paste. The payload is kept, so this can be reversed.
    Disable {
        /// The paste id (a pasted link is reduced to its id; the key is ignored).
        paste: String,
        /// illegal_content, malware, phishing, spam, personal_data, copyright or other.
        #[arg(long)]
        reason: String,
        /// Free-text note for the audit trail. Never shown to anyone else.
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove a paste's payload for good. Cannot be undone.
    Delete {
        paste: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Make a disabled paste available again.
    Restore {
        paste: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Show a paste's lifecycle and moderation history.
    Status {
        paste: String,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the Tokio runtime")?;

    runtime.block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    // Generating a token needs no configuration - it is what you run *before*
    // writing one.
    if matches!(cli.command, Command::Token) {
        return commands::token();
    }

    // The health check needs only the address it probes. It must keep working
    // where validation would fail (no `public_url` in its environment, a
    // database directory it cannot write) and never write anything itself, so
    // the configuration is loaded only for `server.bind`, and not at all with
    // `--address`.
    if let Command::Healthcheck { live, address } = cli.command {
        let address = match address {
            Some(address) => address,
            None => {
                let config = Config::load(cli.config.as_deref()).context("could not load the configuration")?;
                probe_address(&config.server.bind)
            }
        };
        return commands::healthcheck(&address, live).await;
    }

    // Everything else loads and validates first, so a misconfiguration is a
    // startup failure rather than a runtime surprise on the first request.
    let config = Config::load(cli.config.as_deref()).context("could not load the configuration")?;
    let resolved = config.validate().context("configuration is not usable")?;

    match cli.command {
        Command::Token | Command::Healthcheck { .. } => unreachable!("handled above"),
        Command::Serve { bind } => commands::serve(resolved, bind).await,
        Command::Check => commands::check(resolved).await,
        Command::Migrate => commands::migrate(resolved).await,
        Command::Prune { batch_size } => commands::prune(resolved, batch_size).await,
        Command::Backup { output } => commands::backup(resolved, output).await,
        Command::Restore { input, force, dry_run } => commands::restore(resolved, input, force, dry_run).await,
        Command::Reports(command) => match command {
            ReportsCommand::List { status, limit, json } => commands::abuse::reports_list(resolved, status, limit, json).await,
            ReportsCommand::Show { id, json } => commands::abuse::reports_show(resolved, id, json).await,
            ReportsCommand::Review { id } => commands::abuse::reports_review(resolved, id).await,
            ReportsCommand::Close { id, resolution } => commands::abuse::reports_close(resolved, id, resolution).await,
        },
        Command::Moderation(command) => {
            use capsulebin_server::domain::abuse::ModerationAction;
            match command {
                ModerationCommand::Disable { paste, reason, note } => {
                    commands::abuse::moderate(resolved, ModerationAction::Disable, paste, Some(reason), note).await
                }
                ModerationCommand::Delete { paste, reason, note } => {
                    commands::abuse::moderate(resolved, ModerationAction::Delete, paste, Some(reason), note).await
                }
                ModerationCommand::Restore { paste, note } => {
                    commands::abuse::moderate(resolved, ModerationAction::Restore, paste, None, note).await
                }
                ModerationCommand::Status { paste, json } => commands::abuse::moderation_status(resolved, paste, json).await,
            }
        }
    }
}

/// Structured logging.
///
/// Deliberately without request bodies and without the authorization header. The
/// access log carries method, status and duration - never a paste id, because that
/// would put a capability-adjacent value into a file whose retention is longer
/// than the paste's own.
fn init_tracing() {
    use std::io::IsTerminal as _;
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_env("CAPSULEBIN_LOG").unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn"));

    // Logs go to stderr, so stdout carries only a command's own output (`--json`,
    // a generated token) and stays parseable. Colours only on a terminal: a
    // container runtime or journald stores the escape codes verbatim.
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}
