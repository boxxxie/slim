use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::mpsc::Sender,
    time::Instant,
};

use compact_str::CompactString;
use ratatui::layout::Rect;
use tokio::sync::watch;

use crate::{
    client::poller::SentryPollerHandle,
    config::{SlimConfig, save_config},
    domain::SentryMutation,
    effect_registry::EffectRegistry,
    event::SlimEvent,
    id::ProjectScope,
    input::{process_key_event, process_mouse_event},
    notice_service::{NoticeLevel, NoticeMessage, NoticeService},
    notification_buffer::NotificationBuffer,
    stores::SentryStore,
    ui::{
        popup::{config_popup::ConfigPopup, help_popup::HelpPopup},
        stateful_widgets::{ModalState, StatefulWidgets},
        widget::{issue_detail_view::IssueDetailState, notification::NotificationBanner},
    },
};

pub struct SlimApp {
    pub running: bool,
    pub offline: bool,
    pub config_path: PathBuf,
    pub config: SlimConfig,
    pub config_tx: watch::Sender<SlimConfig>,
    pub store: SentryStore,
    pub widgets: StatefulWidgets,
    pub poller: SentryPollerHandle,
    pub notification_buffer: NotificationBuffer,
    pub notice_service: NoticeService,
    pub effects: EffectRegistry,
    pub event_sender: Sender<SlimEvent>,
    pub terminal_area: Rect,
    last_frame_instant: Instant,
}

impl SlimApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config_path: PathBuf,
        config: SlimConfig,
        config_tx: watch::Sender<SlimConfig>,
        store: SentryStore,
        poller: SentryPollerHandle,
        event_sender: Sender<SlimEvent>,
        offline: bool,
    ) -> Self {
        let mut widgets = StatefulWidgets::new(&config);
        widgets.rebuild_derived(&store);
        let effects = EffectRegistry::new(event_sender.clone(), config.effects.clone());

        Self {
            running: true,
            offline,
            config_path,
            config,
            config_tx,
            store,
            widgets,
            poller,
            notification_buffer: NotificationBuffer::new(),
            notice_service: NoticeService::new(),
            effects,
            event_sender,
            terminal_area: Rect::new(0, 0, 120, 40),
            last_frame_instant: Instant::now(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn elapsed_fx_duration(&mut self) -> tachyonfx::Duration {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame_instant);
        self.last_frame_instant = now;
        tachyonfx::Duration::from_millis(elapsed.as_millis() as u32)
    }

    pub fn apply(&mut self, event: SlimEvent, out: &mut Vec<SlimEvent>) {
        self.notice_service.apply(&event);

        match event {
            SlimEvent::InputKey(key) => {
                process_key_event(key, &mut self.widgets, out);
            }
            SlimEvent::InputMouse(mouse) => {
                process_mouse_event(mouse, &mut self.widgets, out);
            }
            SlimEvent::Tick => {}
            SlimEvent::Shutdown => {
                self.store.shutdown();
                self.running = false;
            }
            SlimEvent::ConfigOpen => {
                let area = ConfigPopup::popup_area(self.terminal_area);
                self.effects.register_popup_open(area);
                self.widgets.modal = Some(ModalState::Config(Rc::new(RefCell::new(
                    ConfigPopup::new(self.config.clone()),
                ))));
            }
            SlimEvent::ConfigClose => {
                let area = ConfigPopup::popup_area(self.terminal_area);
                self.effects.register_popup_close(area);
                self.widgets.modal = None;
            }
            SlimEvent::ConfigApply {
                config,
                persist_token,
            } => {
                if let Err(e) = save_config(&self.config_path, &config, persist_token) {
                    self.notice_service.push_notice(
                        NoticeLevel::Error,
                        NoticeMessage::ConfigError(e.to_string().into()),
                    );
                } else {
                    self.config = config.clone();
                    self.effects.effects_config = config.effects.clone();
                    self.widgets.list_state.show_sparklines = config.views.show_sparklines;
                    let _ = self.config_tx.send(config);
                    self.notice_service.push_notice(
                        NoticeLevel::Info,
                        NoticeMessage::GeneralMessage("Saved slim configuration".into()),
                    );
                }
                self.widgets.modal = None;
            }
            SlimEvent::HelpOpen => {
                let area = HelpPopup::popup_area(self.terminal_area);
                self.effects.register_popup_open(area);
                self.widgets.modal =
                    Some(ModalState::Help(Rc::new(RefCell::new(HelpPopup::new()))));
            }
            SlimEvent::HelpClose => {
                let area = HelpPopup::popup_area(self.terminal_area);
                self.effects.register_popup_close(area);
                self.widgets.modal = None;
            }
            SlimEvent::SwitchListView(view) => {
                self.widgets.open_issue_detail = None;
                self.widgets.active_list_view = view;
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::CycleProjectScope { reverse } => {
                let slugs = self.store.project_slugs_sorted();
                let mut scopes = Vec::with_capacity(slugs.len() + 1);
                scopes.push(ProjectScope::All);
                for s in slugs {
                    scopes.push(ProjectScope::Project(s));
                }
                let cur_idx = scopes
                    .iter()
                    .position(|s| s == &self.widgets.filter_scope.project_scope)
                    .unwrap_or(0);
                let next_idx = if reverse {
                    (cur_idx + scopes.len() - 1) % scopes.len()
                } else {
                    (cur_idx + 1) % scopes.len()
                };
                self.widgets.filter_scope.project_scope = scopes[next_idx].clone();
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::CycleIssueSort => {
                self.widgets.filter_scope.sort = self.widgets.filter_scope.sort.next();
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::FilterInputStart => {
                self.widgets.filtering = true;
                self.widgets.filter_input = self.widgets.filter_scope.search_query.to_string();
            }
            SlimEvent::FilterInputChar(ch) => {
                self.widgets.filter_input.push_str(ch.as_str());
                self.widgets.filter_scope.search_query =
                    CompactString::from(self.widgets.filter_input.trim());
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::FilterInputBackspace => {
                self.widgets.filter_input.pop();
                self.widgets.filter_scope.search_query =
                    CompactString::from(self.widgets.filter_input.trim());
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::FilterInputApply(q) => {
                self.widgets.filtering = false;
                self.widgets.filter_scope.search_query = q;
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::FilterInputClear => {
                self.widgets.filtering = false;
                self.widgets.filter_input.clear();
                self.widgets.filter_scope.search_query = CompactString::default();
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::CursorNext => {
                self.widgets
                    .list_state
                    .select_next(self.widgets.active_list_view);
            }
            SlimEvent::CursorPrevious => {
                self.widgets
                    .list_state
                    .select_prev(self.widgets.active_list_view);
            }
            SlimEvent::CursorPageDown => {
                let step = (self.terminal_area.height.saturating_sub(6) / 2).max(5) as usize;
                self.widgets
                    .list_state
                    .select_page_down(self.widgets.active_list_view, step);
            }
            SlimEvent::CursorPageUp => {
                let step = (self.terminal_area.height.saturating_sub(6) / 2).max(5) as usize;
                self.widgets
                    .list_state
                    .select_page_up(self.widgets.active_list_view, step);
            }
            SlimEvent::CursorFirst => {
                self.widgets
                    .list_state
                    .select_first(self.widgets.active_list_view);
            }
            SlimEvent::CursorLast => {
                self.widgets
                    .list_state
                    .select_last(self.widgets.active_list_view);
            }
            SlimEvent::SelectAndOpenIssue {
                row_index,
                issue_id,
            } => {
                self.widgets
                    .list_state
                    .table_state
                    .select(Some(row_index));
                out.push(SlimEvent::OpenIssueDetail(issue_id));
            }
            SlimEvent::EscapeOrBack => {
                if self.widgets.open_issue_detail.is_some() {
                    self.widgets.open_issue_detail = None;
                } else if !self.widgets.filter_scope.search_query.is_empty() {
                    out.push(SlimEvent::FilterInputClear);
                } else if self.widgets.active_notice.is_some() {
                    self.widgets.active_notice = None;
                }
            }
            SlimEvent::OpenSelectedIssue => {
                if let Some(issue_id) = self
                    .widgets
                    .list_state
                    .selected_issue_id(self.widgets.active_list_view)
                {
                    out.push(SlimEvent::OpenIssueDetail(issue_id));
                }
            }
            SlimEvent::OpenIssueDetail(issue_id) => {
                let needs_fetch = match (
                    self.store.issues.get(&issue_id),
                    self.store.events_by_issue.get(&issue_id),
                ) {
                    (Some(iss), Some(ev)) => ev.date_created < iss.last_seen,
                    (Some(_), None) => true,
                    _ => false,
                };
                let loading = needs_fetch && !self.offline;
                if loading {
                    self.poller.fetch_issue_event(issue_id.clone());
                }
                self.widgets.open_issue_detail = Some(IssueDetailState::new(issue_id, loading));
            }
            SlimEvent::CloseIssueDetail => {
                self.widgets.open_issue_detail = None;
            }
            SlimEvent::SwitchDetailPage(page) => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.switch_page(page);
                }
            }
            SlimEvent::NextDetailPage => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.switch_page(detail.active_page.next());
                }
            }
            SlimEvent::PrevDetailPage => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.switch_page(detail.active_page.prev());
                }
            }
            SlimEvent::DetailScrollDown(n) => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.scroll_down(n);
                }
            }
            SlimEvent::DetailScrollUp(n) => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.scroll_up(n);
                }
            }
            SlimEvent::DetailScrollTop => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.set_scroll(0);
                }
            }
            SlimEvent::DetailScrollBottom => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.set_scroll(u16::MAX);
                }
            }
            SlimEvent::ToggleInAppFramesOnly => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.in_app_frames_only = !detail.in_app_frames_only;
                    detail.set_scroll(0);
                }
            }
            SlimEvent::ToggleExpandFrameVars => {
                if let Some(ref mut detail) = self.widgets.open_issue_detail {
                    detail.expand_frame_vars = !detail.expand_frame_vars;
                }
            }
            SlimEvent::MutateSelectedStatus(status) => {
                if let Some(issue_id) = self.widgets.current_target_issue_id() {
                    if let Some(issue) = self.store.issues.get(&issue_id) {
                        out.push(SlimEvent::SubmitMutation(SentryMutation::UpdateStatus {
                            issue_id,
                            short_id: issue.short_id.clone(),
                            status,
                        }));
                    }
                }
            }
            SlimEvent::SubmitMutation(mutation) => {
                if self.offline {
                    self.notice_service.push_notice(
                        NoticeLevel::Error,
                        NoticeMessage::GeneralMessage(
                            "Cannot mutate Sentry issue status in offline mode".into(),
                        ),
                    );
                } else {
                    self.poller.execute_mutation(mutation);
                }
            }
            SlimEvent::IssueStatusUpdated {
                issue_id,
                short_id,
                status,
            } => {
                self.store.apply_status_mutation(&issue_id, status);
                self.widgets.rebuild_derived(&self.store);
                self.notice_service.push_notice(
                    NoticeLevel::Info,
                    NoticeMessage::GeneralMessage(
                        format!("Updated {short_id} → {}", status.label()).into(),
                    ),
                );
            }
            SlimEvent::CopySelectedShortId => {
                if let Some(issue_id) = self.widgets.current_target_issue_id() {
                    if let Some(issue) = self.store.issues.get(&issue_id) {
                        out.push(SlimEvent::CopyToClipboard {
                            label: CompactString::from("Short ID"),
                            value: issue.short_id.to_string(),
                        });
                    }
                }
            }
            SlimEvent::CopySelectedMarkdownSummary => {
                if let Some(issue_id) = self.widgets.current_target_issue_id() {
                    if let Some(issue) = self.store.issues.get(&issue_id) {
                        let md = if let Some(ev) = self.store.events_by_issue.get(&issue_id) {
                            ev.format_markdown_summary(issue)
                        } else {
                            format!(
                                "# [{}] {}\n- **Project:** `{}` | **Level:** `{}` | **Status:** `{}`\n- **Events:** {} | **URL:** {}\n",
                                issue.short_id,
                                issue.title,
                                issue.project_slug,
                                issue.level.label(),
                                issue.status.label(),
                                issue.count,
                                issue.permalink
                            )
                        };
                        out.push(SlimEvent::CopyToClipboard {
                            label: CompactString::from("Markdown summary"),
                            value: md,
                        });
                    }
                }
            }
            SlimEvent::OpenSelectedUrl => {
                if let Some(issue_id) = self.widgets.current_target_issue_id() {
                    if let Some(issue) = self.store.issues.get(&issue_id) {
                        if !issue.permalink.is_empty() {
                            out.push(SlimEvent::OpenUrl(issue.permalink.clone()));
                        }
                    }
                }
            }
            SlimEvent::CopyToClipboard { label, value } => {
                if copy_text_to_clipboard(&value) {
                    self.notice_service.push_notice(
                        NoticeLevel::Info,
                        NoticeMessage::GeneralMessage(format!("Copied {label} to clipboard").into()),
                    );
                } else {
                    self.notice_service.push_notice(
                        NoticeLevel::Error,
                        NoticeMessage::GeneralMessage("Failed to access system clipboard".into()),
                    );
                }
            }
            SlimEvent::OpenUrl(url) => {
                let _ = open::that(url.as_str());
                self.notice_service.push_notice(
                    NoticeLevel::Info,
                    NoticeMessage::GeneralMessage(format!("Opened {url} in browser").into()),
                );
            }
            SlimEvent::ManualRefresh => {
                if self.offline {
                    self.notice_service.push_notice(
                        NoticeLevel::Info,
                        NoticeMessage::GeneralMessage("Offline mode active".into()),
                    );
                } else {
                    self.poller.sync_now();
                    if let Some(ref mut detail) = self.widgets.open_issue_detail {
                        detail.loading_event = true;
                        self.poller.fetch_issue_event(detail.issue_id.clone());
                    }
                    self.notice_service.push_notice(
                        NoticeLevel::Info,
                        NoticeMessage::GeneralMessage("Refreshing from Sentry...".into()),
                    );
                }
            }
            SlimEvent::PollBatchLoaded(payload) => {
                let changes = self.store.apply_poll_batch(
                    *payload,
                    &self.config.notifications,
                    self.config.polling.max_stored_issues,
                    self.config.polling.max_activity_events,
                );
                self.notification_buffer.enqueue(changes);
                self.widgets.rebuild_derived(&self.store);
            }
            SlimEvent::IssueEventLoaded(detail) => {
                let loaded_id = detail.issue_id.clone();
                self.store.apply_issue_event(*detail);
                if let Some(ref mut d_state) = self.widgets.open_issue_detail {
                    if d_state.issue_id == loaded_id {
                        d_state.loading_event = false;
                    }
                }
            }
            SlimEvent::FlushNotificationBuffer => {
                let alerts = self
                    .notification_buffer
                    .evaluate_and_flush(&self.config.notifications, !self.offline);
                for alert in alerts {
                    self.notice_service.push_notice(
                        NoticeLevel::Info,
                        NoticeMessage::GeneralMessage(
                            format!("{} — {}", alert.summary, alert.body).into(),
                        ),
                    );
                }
            }
            SlimEvent::NotificationDismiss => {
                self.widgets.active_notice = None;
            }
            SlimEvent::NotificationRecallLast => {
                if let Some(last) = self.notice_service.last_notification().cloned() {
                    let banner_rect = NotificationBanner::banner_area(self.terminal_area);
                    self.effects.register_notification_effect(banner_rect);
                    self.widgets.active_notice = Some(last);
                }
            }
            SlimEvent::SyncError(_)
            | SlimEvent::MutationFailed(_)
            | SlimEvent::AppError(_)
            | SlimEvent::ActivityNotice(_)
            | SlimEvent::LogEntry(_)
            | SlimEvent::LogLevelChanged(_) => {}
        }

        if self.widgets.active_notice.is_none() || self.notice_service.has_error() {
            if let Some(next_notice) = self.notice_service.pop_notice() {
                let banner_rect = NotificationBanner::banner_area(self.terminal_area);
                self.effects.register_notification_effect(banner_rect);
                self.widgets.active_notice = Some(next_notice);
            }
        }
    }
}

fn copy_text_to_clipboard(text: &str) -> bool {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        if cb.set_text(text.to_string()).is_ok() {
            return true;
        }
    }
    for (bin, args) in [
        ("wl-copy", &[][..]),
        ("xclip", &["-selection", "clipboard"][..]),
    ] {
        if let Ok(mut child) = std::process::Command::new(bin)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = std::io::Write::write_all(&mut stdin, text.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return true;
                }
            }
        }
    }
    false
}
