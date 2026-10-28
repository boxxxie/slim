use std::{path::PathBuf, sync::mpsc::Sender};

use tokio::sync::watch;

use crate::{
    client::{
        config::ClientConfig,
        poller::spawn_sentry_poller,
        service::SentryService,
    },
    config::{default_db_path, load_config},
    event::SlimEvent,
    notification_buffer::spawn_notification_buffer_timer,
    result::Result,
    slim_app::SlimApp,
    stores::SentryStore,
};

pub fn initialize_app(
    config_path: PathBuf,
    sender: Sender<SlimEvent>,
    offline: bool,
) -> Result<SlimApp> {
    let config = load_config(&config_path)?;
    let (config_tx, config_rx) = watch::channel(config.clone());

    let mut store = SentryStore::new(default_db_path());
    store.init_duckdb_and_load(&config.database, Some(&sender));

    let client_cfg = ClientConfig::from_slim_config(&config, offline);
    let service = SentryService::new(client_cfg)?;
    let poller = spawn_sentry_poller(service, sender.clone(), config_rx.clone(), offline);

    spawn_notification_buffer_timer(sender.clone(), config_rx);

    Ok(SlimApp::new(
        config_path,
        config,
        config_tx,
        store,
        poller,
        sender,
        offline,
    ))
}
