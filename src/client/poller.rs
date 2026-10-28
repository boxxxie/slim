use std::{
    sync::{Arc, mpsc::Sender},
    time::Duration,
};

use tokio::sync::{Mutex, Semaphore, mpsc as tokio_mpsc, watch};

use crate::{
    client::{config::ClientConfig, error::ClientError, service::SentryService},
    config::SlimConfig,
    dispatcher::Dispatcher,
    domain::SentryMutation,
    event::SlimEvent,
    id::IssueId,
    result::SlimError,
};

#[derive(Debug, Clone)]
pub enum PollerCommand {
    SyncNow,
    FetchIssueEvent(IssueId),
    ExecuteMutation(SentryMutation),
}

#[derive(Debug, Clone)]
pub struct SentryPollerHandle {
    cmd_tx: tokio_mpsc::UnboundedSender<PollerCommand>,
}

impl SentryPollerHandle {
    pub fn sync_now(&self) {
        let _ = self.cmd_tx.send(PollerCommand::SyncNow);
    }

    pub fn fetch_issue_event(&self, issue_id: IssueId) {
        let _ = self.cmd_tx.send(PollerCommand::FetchIssueEvent(issue_id));
    }

    pub fn execute_mutation(&self, mutation: SentryMutation) {
        let _ = self.cmd_tx.send(PollerCommand::ExecuteMutation(mutation));
    }
}

pub fn spawn_sentry_poller(
    service: SentryService,
    sender: Sender<SlimEvent>,
    mut config_rx: watch::Receiver<SlimConfig>,
    offline: bool,
) -> SentryPollerHandle {
    let (cmd_tx, mut cmd_rx) = tokio_mpsc::unbounded_channel::<PollerCommand>();
    let service = Arc::new(Mutex::new(service));
    let sync_gate = Arc::new(Semaphore::new(1));

    tokio::spawn(async move {
        let initial_cfg = config_rx.borrow().clone();
        let mut poll_secs = initial_cfg.polling.poll_seconds.max(15);

        let mut poll_ticker = tokio::time::interval(Duration::from_secs(poll_secs));
        poll_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        poll_ticker.tick().await;

        if !offline {
            run_sync_guarded(&service, &sync_gate, &sender).await;
        }

        loop {
            tokio::select! {
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        PollerCommand::SyncNow => {
                            if !offline {
                                run_sync_guarded(&service, &sync_gate, &sender).await;
                            }
                        }
                        PollerCommand::FetchIssueEvent(issue_id) => {
                            if !offline {
                                let svc = service.lock().await;
                                if let Err(err) = svc.fetch_issue_event(&issue_id, &sender).await {
                                    handle_client_error(err, &sender);
                                }
                            }
                        }
                        PollerCommand::ExecuteMutation(mutation) => {
                            if !offline {
                                let svc = service.lock().await;
                                if let Err(err) = svc.execute_mutation(mutation, &sender).await {
                                    sender.dispatch(SlimEvent::MutationFailed(err.to_string().into()));
                                }
                            }
                        }
                    }
                }
                _ = poll_ticker.tick() => {
                    if !offline {
                        run_sync_guarded(&service, &sync_gate, &sender).await;
                    }
                }
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let next_cfg = config_rx.borrow_and_update().clone();
                    let next_poll = next_cfg.polling.poll_seconds.max(15);

                    {
                        let mut svc = service.lock().await;
                        svc.update_config(ClientConfig::from_slim_config(&next_cfg, offline));
                    }

                    if next_poll != poll_secs {
                        poll_secs = next_poll;
                        poll_ticker = tokio::time::interval(Duration::from_secs(poll_secs));
                        poll_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        poll_ticker.tick().await;
                    }

                    if !offline {
                        run_sync_guarded(&service, &sync_gate, &sender).await;
                    }
                }
            }
        }
    });

    SentryPollerHandle { cmd_tx }
}

async fn run_sync_guarded(
    service: &Arc<Mutex<SentryService>>,
    sync_gate: &Arc<Semaphore>,
    sender: &Sender<SlimEvent>,
) {
    let Ok(_permit) = sync_gate.try_acquire() else {
        return;
    };
    let mut svc = service.lock().await;
    if let Err(err) = svc.sync(sender).await {
        handle_client_error(err, sender);
    }
}

fn handle_client_error(err: ClientError, sender: &Sender<SlimEvent>) {
    match err {
        ClientError::OfflineMode => {}
        ClientError::Authentication => {
            sender.dispatch(SlimEvent::AppError(SlimError::InvalidSentryToken));
        }
        other => {
            sender.dispatch(SlimEvent::SyncError(other.to_string().into()));
        }
    }
}
