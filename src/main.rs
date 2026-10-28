#![recursion_limit = "256"]

pub mod app_init;
pub mod client;
pub mod config;
pub mod dispatcher;
pub mod domain;
pub mod effect_registry;
pub mod event;
pub mod gruvbox;
pub mod id;
pub mod input;
pub mod logging;
pub mod notice_service;
pub mod notification_buffer;
pub mod os_notify;
pub mod rendering;
pub mod result;
pub mod slim_app;
pub mod stores;
pub mod theme;
pub mod tui;
pub mod ui;

use std::{io, path::PathBuf, sync::mpsc, time::Duration};

use clap::Parser;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app_init::initialize_app,
    client::{config::ClientConfig, service::SentryService},
    config::{
        SecretSource, default_config_path, default_db_path, load_config,
        resolve_auth_token,
    },
    event::{EventHandler, SlimEvent},
    logging::{LoggingConfig, init_logging},
    rendering::render_main_ui,
    result::Result,
    stores::{SentryStore, load_or_create_quack_token, resolve_duckdb_binary},
    tui::Tui,
};

#[derive(Debug, Parser)]
#[command(
    name = "slim",
    version,
    about = "Single-page Sentry TUI monitor & multi-page issue inspector backed by DuckDB 2.0 and the Quack protocol"
)]
struct CliArgs {
    /// Custom path to slim.toml configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Print the default configuration file path and exit
    #[arg(long)]
    print_config_path: bool,

    /// Verify configuration, Sentry token resolution, and DuckDB 2.0 availability and exit
    #[arg(long)]
    verify_config: bool,

    /// Print the DuckDB 2.0 Quack protocol ATTACH snippet for concurrent CLI queries and exit
    #[arg(long)]
    print_quack_attach: bool,

    /// Run a single non-interactive sync into DuckDB 2.0 and exit without launching the TUI
    #[arg(long)]
    sync_once: bool,

    /// Non-interactively inspect an issue by Short ID (e.g. ACME-API-42) or Issue ID and exit
    #[arg(long, value_name = "ISSUE")]
    inspect: Option<String>,

    /// Launch in offline mode using cached DuckDB 2.0 rows without polling Sentry
    #[arg(long)]
    offline: bool,

    /// Enable verbose debug logging
    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();
    let config_path = args.config.unwrap_or_else(default_config_path);

    if args.print_config_path {
        println!("{}", config_path.display());
        return Ok(());
    }

    if args.print_quack_attach {
        let cfg = load_config(&config_path).unwrap_or_default();
        let token = load_or_create_quack_token();
        println!(
            "LOAD quack;\nCREATE SECRET (TYPE quack, TOKEN '{}');\nATTACH 'quack:127.0.0.1:{}' AS slim;\nSELECT short_id, project_slug, level, status, count, last_seen, title FROM slim.issues ORDER BY last_seen DESC LIMIT 20;",
            token, cfg.database.quack_port
        );
        return Ok(());
    }

    if args.verify_config {
        let cfg = load_config(&config_path)?;
        let token_status = match resolve_auth_token(&cfg) {
            Some(src) => {
                let kind = match src {
                    SecretSource::EnvVar(_) => "SENTRY_AUTH_TOKEN env var",
                    SecretSource::ConfigFile(_) => "slim.toml [sentry].auth_token",
                    SecretSource::SentryCliFallback(_) => "~/.sentryclirc [auth] token fallback",
                };
                format!("resolved ({kind})")
            }
            None => "missing (set SENTRY_AUTH_TOKEN, ~/.sentryclirc, or configure via 'c' in slim)"
                .to_string(),
        };
        let duckdb_bin = resolve_duckdb_binary();
        let db_path = default_db_path();
        let projects = if cfg.sentry.projects.is_empty() {
            "all org projects".to_string()
        } else {
            cfg.sentry
                .projects
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!("config_path: {}", config_path.display());
        println!("duckdb_binary: {}", duckdb_bin.display());
        println!("database_path: {}", db_path.display());
        println!(
            "quack_server: {} (127.0.0.1:{})",
            if cfg.database.quack_server_enabled {
                "enabled"
            } else {
                "disabled"
            },
            cfg.database.quack_port
        );
        println!("org: {}", cfg.sentry.org);
        println!("projects: [{projects}]");
        println!("auth_token: {token_status}");
        return Ok(());
    }

    if args.sync_once {
        return run_sync_once(config_path).await;
    }

    if let Some(target) = args.inspect {
        return run_inspect_issue(config_path, &target, args.offline).await;
    }

    let events = EventHandler::new(Duration::from_millis(33));
    let sender = events.sender();
    let _log_guard = init_logging(LoggingConfig::from_env(args.debug), Some(sender.clone()));

    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)
        .map_err(|e| result::SlimError::GeneralError(e.to_string().into()))?;
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    let mut app = initialize_app(config_path, sender, args.offline)?;
    let mut ui_events = Vec::with_capacity(8);

    while app.is_running() {
        let elapsed = app.elapsed_fx_duration();
        let org_slug = app.config.sentry.org.clone();
        tui.draw_frame(|frame| {
            app.terminal_area = frame.area();
            render_main_ui(
                frame,
                &mut app.widgets,
                &app.store,
                org_slug.as_str(),
                app.offline,
            );
            let area = frame.area();
            app.effects
                .process_effects(elapsed, frame.buffer_mut(), area);
        })?;

        tui.events.set_fast_tick(app.effects.is_running());

        if let Ok(ev) = tui.events.next() {
            app.apply(ev, &mut ui_events);
            while let Some(next_ev) = tui.events.try_next() {
                app.apply(next_ev, &mut ui_events);
            }
            while !ui_events.is_empty() {
                let batch = std::mem::take(&mut ui_events);
                for derived in batch {
                    app.apply(derived, &mut ui_events);
                }
            }
        }
    }

    tui.exit()?;
    Ok(())
}

async fn run_sync_once(config_path: PathBuf) -> Result<()> {
    let cfg = load_config(&config_path)?;
    let mut store = SentryStore::new(default_db_path());
    let mut db_cfg = cfg.database.clone();
    db_cfg.quack_server_enabled = false;
    store.init_duckdb_and_load(&db_cfg, None);

    let (tx, rx) = mpsc::channel();
    let client_cfg = ClientConfig::from_slim_config(&cfg, false);
    let mut service = SentryService::new(client_cfg)?;
    service.sync(&tx).await?;

    // Also hydrate the latest event for the top 5 most recent issues so offline/CLI inspection has event details ready
    let mut top_issue_ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let SlimEvent::PollBatchLoaded(payload) = ev {
            for iss in payload.issues.iter().take(5) {
                top_issue_ids.push(iss.id.clone());
            }
            store.apply_poll_batch(
                *payload,
                &cfg.notifications,
                cfg.polling.max_stored_issues,
                cfg.polling.max_activity_events,
            );
        }
    }

    for issue_id in top_issue_ids {
        if service.fetch_issue_event(&issue_id, &tx).await.is_ok() {
            while let Ok(ev) = rx.try_recv() {
                if let SlimEvent::IssueEventLoaded(detail) = ev {
                    store.apply_issue_event(*detail);
                }
            }
        }
    }

    store.shutdown();
    tokio::time::sleep(Duration::from_millis(250)).await;

    println!(
        "Synced projects={} issues={} cached_events={} activity_events={}",
        store.projects.len(),
        store.issues.len(),
        store.events_by_issue.len(),
        store.activity_events.len()
    );
    Ok(())
}

async fn run_inspect_issue(config_path: PathBuf, target: &str, offline: bool) -> Result<()> {
    let cfg = load_config(&config_path)?;
    let mut store = SentryStore::new(default_db_path());
    let mut db_cfg = cfg.database.clone();
    db_cfg.quack_server_enabled = false;
    store.init_duckdb_and_load(&db_cfg, None);

    let trimmed = target.trim();
    let matched_id = store
        .issues
        .values()
        .find(|i| {
            i.short_id.eq_ignore_ascii_case(trimmed) || i.id.as_str().eq_ignore_ascii_case(trimmed)
        })
        .map(|i| i.id.clone());

    let Some(issue_id) = matched_id else {
        store.shutdown();
        return Err(result::SlimError::GeneralError(
            format!("Issue '{trimmed}' not found in local DuckDB cache (run `slim --sync-once` first)").into(),
        ));
    };

    if !offline && !store.events_by_issue.contains_key(&issue_id) {
        let (tx, rx) = mpsc::channel();
        let client_cfg = ClientConfig::from_slim_config(&cfg, false);
        let service = SentryService::new(client_cfg)?;
        if service.fetch_issue_event(&issue_id, &tx).await.is_ok() {
            while let Ok(ev) = rx.try_recv() {
                if let SlimEvent::IssueEventLoaded(detail) = ev {
                    store.apply_issue_event(*detail);
                }
            }
        }
    }

    if let Some(issue) = store.issues.get(&issue_id) {
        if let Some(ev) = store.events_by_issue.get(&issue_id) {
            print!("{}", ev.format_markdown_summary(issue));
        } else {
            println!(
                "# [{}] {}\n- **Project:** `{}` | **Level:** `{}` | **Status:** `{}`\n- **Events:** {} | **URL:** {}",
                issue.short_id,
                issue.title,
                issue.project_slug,
                issue.level.label(),
                issue.status.label(),
                issue.count,
                issue.permalink
            );
        }
    }

    store.shutdown();
    tokio::time::sleep(Duration::from_millis(150)).await;
    Ok(())
}
