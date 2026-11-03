use std::{sync::mpsc::Sender, time::Duration};

use chrono::Utc;
use compact_str::CompactString;
use tokio::sync::watch;

use crate::{
    config::{NotificationsSection, SlimConfig},
    dispatcher::Dispatcher,
    domain::{
        DomainChange, NotificationEmission, NotificationFilterMode, UserInteraction,
    },
    event::SlimEvent,
    id::IssueId,
    os_notify::{NotifyUrgency, send_os_notification},
    relevance::{NotificationEvaluationResult, NotificationRelevanceContext, evaluate_notifications},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedNotification {
    pub summary: CompactString,
    pub body: CompactString,
    pub urgency: NotifyUrgency,
    pub url: Option<CompactString>,
}

#[derive(Debug, Default)]
pub struct NotificationBuffer {
    queued: Vec<DomainChange>,
    flush_nonce: u64,
    pub total_emitted: u64,
    pub total_suppressed: u64,
}

impl NotificationBuffer {
    pub fn new() -> Self {
        Self {
            queued: Vec::new(),
            flush_nonce: 0,
            total_emitted: 0,
            total_suppressed: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    pub fn len(&self) -> usize {
        self.queued.len()
    }

    pub fn enqueue(&mut self, changes: impl IntoIterator<Item = DomainChange>) {
        self.queued.extend(changes);
    }

    /// Purges any pending un-flushed changes for an issue when the user resolves or ignores it.
    pub fn drop_issue(&mut self, issue_id: &IssueId) {
        self.queued.retain(|change| &change.issue_id != issue_id);
    }

    pub fn evaluate_and_flush(
        &mut self,
        config: &NotificationsSection,
        recent_emissions: &[NotificationEmission],
        user_interactions: &[UserInteraction],
        filter_mode: NotificationFilterMode,
        send_desktop: bool,
    ) -> NotificationEvaluationResult {
        if self.queued.is_empty() {
            return NotificationEvaluationResult::default();
        }

        let drained = std::mem::take(&mut self.queued);
        self.flush_nonce = self.flush_nonce.wrapping_add(1);

        let ctx = NotificationRelevanceContext {
            config,
            recent_emissions,
            user_interactions,
            filter_mode,
            now: Utc::now(),
            flush_nonce: self.flush_nonce,
        };

        let result = evaluate_notifications(drained, &ctx);
        self.total_emitted = self
            .total_emitted
            .saturating_add(result.emitted.len() as u64);
        self.total_suppressed = self
            .total_suppressed
            .saturating_add(result.suppressed_count as u64);

        if send_desktop && config.desktop_notifications_enabled {
            for alert in &result.emitted {
                send_os_notification(
                    alert.summary.as_str(),
                    alert.body.as_str(),
                    alert.urgency,
                    6000,
                    alert.url.as_deref(),
                );
            }
        }

        result
    }
}

pub fn spawn_notification_buffer_timer(
    sender: Sender<SlimEvent>,
    mut config_rx: watch::Receiver<SlimConfig>,
) {
    tokio::spawn(async move {
        let mut current_secs = config_rx
            .borrow()
            .notifications
            .notification_check_seconds
            .max(2);
        let mut ticker = tokio::time::interval(Duration::from_secs(current_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    sender.dispatch(SlimEvent::FlushNotificationBuffer);
                }
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let next_secs = config_rx
                        .borrow_and_update()
                        .notifications
                        .notification_check_seconds
                        .max(2);
                    if next_secs != current_secs {
                        current_secs = next_secs;
                        ticker = tokio::time::interval(Duration::from_secs(current_secs));
                        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        ticker.tick().await;
                    }
                }
            }
        }
    });
}
