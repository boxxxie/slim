use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc::Sender,
    time::Duration,
};

use compact_str::CompactString;
use tokio::sync::watch;

use crate::{
    config::{NotificationsSection, SlimConfig},
    dispatcher::Dispatcher,
    domain::DomainChange,
    event::SlimEvent,
    id::IssueId,
    os_notify::{send_os_notification, NotifyUrgency},
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
}

impl NotificationBuffer {
    pub fn new() -> Self {
        Self { queued: Vec::new() }
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

    pub fn evaluate_and_flush(
        &mut self,
        config: &NotificationsSection,
        send_desktop: bool,
    ) -> Vec<EmittedNotification> {
        if self.queued.is_empty() {
            return Vec::new();
        }

        let drained = std::mem::take(&mut self.queued);

        let mut by_issue: BTreeMap<IssueId, Vec<DomainChange>> = BTreeMap::new();
        for change in drained {
            by_issue
                .entry(change.issue_id.clone())
                .or_default()
                .push(change);
        }

        let mut emitted = Vec::new();
        let mut standard_groups: Vec<(IssueId, Vec<DomainChange>)> = Vec::new();

        for (issue_id, group) in by_issue {
            let is_critical =
                config.critical_unhandled_alert && group.iter().any(|c| c.is_critical);

            if is_critical {
                let first = &group[0];
                let summaries: Vec<&str> = group.iter().map(|c| c.summary.as_str()).collect();
                let combined_summary = deduplicate_summaries(&summaries);
                emitted.push(EmittedNotification {
                    summary: format!(
                        "[{} • {}] {} — {}",
                        first.project_slug,
                        first.level.label(),
                        first.short_id,
                        first.issue_title
                    )
                    .into(),
                    body: combined_summary.into(),
                    urgency: NotifyUrgency::Critical,
                    url: Some(first.url.clone()),
                });
            } else {
                standard_groups.push((issue_id, group));
            }
        }

        let coalesce_threshold = config.coalesce_threshold.max(1);
        if standard_groups.len() > coalesce_threshold {
            let issue_count = standard_groups.len();
            let projects: BTreeSet<&str> = standard_groups
                .iter()
                .filter_map(|(_, g)| g.first().map(|c| c.project_slug.as_str()))
                .collect();
            let total_delta: u64 = standard_groups
                .iter()
                .flat_map(|(_, g)| g.iter().map(|c| c.delta_events))
                .sum();

            let proj_label = if projects.is_empty() {
                "Sentry".to_string()
            } else {
                projects.into_iter().take(4).collect::<Vec<_>>().join(", ")
            };
            let summary = if total_delta > 0 {
                format!("{issue_count} Sentry issues active in {proj_label} (+{total_delta} events)")
            } else {
                format!("{issue_count} Sentry issues updated in {proj_label}")
            };

            let identifiers: Vec<&str> = standard_groups
                .iter()
                .filter_map(|(_, g)| g.first().map(|c| c.short_id.as_str()))
                .take(8)
                .collect();

            emitted.push(EmittedNotification {
                summary: summary.into(),
                body: format!("Issues: {}", identifiers.join(", ")).into(),
                urgency: NotifyUrgency::Normal,
                url: None,
            });
        } else {
            for (_issue_id, group) in standard_groups {
                if config.per_issue_dedup {
                    let first = &group[0];
                    let summaries: Vec<&str> = group.iter().map(|c| c.summary.as_str()).collect();
                    let combined_summary = deduplicate_summaries(&summaries);
                    emitted.push(EmittedNotification {
                        summary: format!(
                            "[{}] {}: {}",
                            first.project_slug, first.short_id, first.issue_title
                        )
                        .into(),
                        body: combined_summary.into(),
                        urgency: NotifyUrgency::Normal,
                        url: Some(first.url.clone()),
                    });
                } else {
                    for change in group {
                        emitted.push(EmittedNotification {
                            summary: format!(
                                "[{}] {}: {}",
                                change.project_slug, change.short_id, change.issue_title
                            )
                            .into(),
                            body: change.summary,
                            urgency: NotifyUrgency::Normal,
                            url: Some(change.url),
                        });
                    }
                }
            }
        }

        if send_desktop && config.desktop_notifications_enabled {
            for alert in &emitted {
                send_os_notification(
                    alert.summary.as_str(),
                    alert.body.as_str(),
                    alert.urgency,
                    6000,
                    alert.url.as_deref(),
                );
            }
        }

        emitted
    }
}

fn deduplicate_summaries(summaries: &[&str]) -> String {
    let mut unique: Vec<&str> = Vec::new();
    for &s in summaries {
        if !unique.contains(&s) {
            unique.push(s);
        }
    }
    unique.join("; ")
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
