use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Paragraph, StatefulWidget, Widget},
};

use crate::{
    domain::{HydrationState, IssueLevel, IssueStatus, ListViewKind},
    event::SlimEvent,
    stores::SentryStore,
    theme::theme,
    ui::{
        stateful_widgets::{ModalState, StatefulWidgets},
        widget::{
            input_field::FilterInputBar, issue_detail_view::IssueDetailView,
            issues_table::IssuesTableWidget, notification::NotificationBanner,
        },
    },
};

pub fn render_main_ui(
    frame: &mut Frame,
    widgets: &mut StatefulWidgets,
    store: &SentryStore,
    org_slug: &str,
    offline: bool,
) {
    let area = frame.area();
    let buf = frame.buffer_mut();

    widgets.clear_hitboxes();

    Block::default().style(theme().background).render(area, buf);

    let show_filter_bar = widgets.filtering || !widgets.filter_scope.search_query.is_empty();

    let mut constraints = Vec::with_capacity(4);
    constraints.push(Constraint::Length(1)); // Top header bar
    if show_filter_bar && !widgets.is_in_detail_view() {
        constraints.push(Constraint::Length(1)); // Filter bar
    }
    constraints.push(Constraint::Min(8)); // Full-screen single-page body (List OR Multi-page Detail)
    constraints.push(Constraint::Length(1)); // Shortcuts footer bar

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;
    let header_area = chunks[idx];
    idx += 1;

    let filter_area = if show_filter_bar && !widgets.is_in_detail_view() {
        let r = chunks[idx];
        idx += 1;
        Some(r)
    } else {
        None
    };

    let body_area = chunks[idx];
    idx += 1;
    let footer_area = chunks[idx];

    render_top_chrome(header_area, buf, widgets, store, org_slug, offline);

    if let Some(f_area) = filter_area {
        let val = if widgets.filtering {
            widgets.filter_input.as_str()
        } else {
            widgets.filter_scope.search_query.as_str()
        };
        FilterInputBar::new(val, widgets.filtering).render(f_area, buf);
        widgets.register_hitbox(f_area, SlimEvent::FilterInputClear);
    }

    let focused = widgets.modal.is_none();

    if let Some(ref mut detail_state) = widgets.open_issue_detail {
        IssueDetailView::new(store, detail_state, &mut widgets.hitboxes).render(body_area, buf);
    } else {
        IssuesTableWidget::new(widgets.active_list_view, focused, &mut widgets.hitboxes).render(
            body_area,
            buf,
            &mut widgets.list_state,
        );
    }

    let shortcuts = widgets.active_shortcuts();
    Paragraph::new(shortcuts.as_line()).render(footer_area, buf);

    if let Some(ref modal) = widgets.modal {
        match modal {
            ModalState::Config(c) => c.borrow().render_in(area, buf),
            ModalState::Help(h) => h.borrow().render_in(area, buf),
        }
    }

    if let Some(ref notice) = widgets.active_notice {
        NotificationBanner::new(notice).render(area, buf);
        widgets.register_hitbox(
            NotificationBanner::banner_area(area),
            SlimEvent::NotificationDismiss,
        );
    }
}

fn push_clickable_span<'a>(
    spans: &mut Vec<Span<'a>>,
    hitboxes: &mut Vec<(Rect, SlimEvent)>,
    cur_x: &mut u16,
    y: u16,
    max_x: u16,
    span: Span<'a>,
    event: Option<SlimEvent>,
) {
    let w = span.width() as u16;
    if let Some(ev) = event {
        let clamped_w = w.min(max_x.saturating_sub(*cur_x));
        if clamped_w > 0 {
            hitboxes.push((
                Rect {
                    x: *cur_x,
                    y,
                    width: clamped_w,
                    height: 1,
                },
                ev,
            ));
        }
    }
    *cur_x = cur_x.saturating_add(w);
    spans.push(span);
}

fn render_top_chrome(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    widgets: &mut StatefulWidgets,
    store: &SentryStore,
    org_slug: &str,
    offline: bool,
) {
    let unresolved_count = store
        .issues
        .values()
        .filter(|i| i.status == IssueStatus::Unresolved)
        .count();
    let high_unhandled_count = store
        .issues
        .values()
        .filter(|i| {
            i.status == IssueStatus::Unresolved
                && (i.is_unhandled
                    || i.level == IssueLevel::Fatal
                    || i.priority == crate::domain::IssuePriority::High
                    || i.substatus.as_deref() == Some("regressed")
                    || i.substatus.as_deref() == Some("escalating"))
        })
        .count();
    let feed_count = store.activity_events.len();
    let all_count = store.issues.len();

    let in_detail = widgets.is_in_detail_view();
    let active_list_view = widgets.active_list_view;
    let tab_style = |v: ListViewKind| {
        if !in_detail && active_list_view == v {
            theme().active_tab
        } else {
            theme().inactive_tab
        }
    };

    let sync_label = if offline {
        "OFFLINE".to_string()
    } else {
        match store.hydration_state {
            HydrationState::ColdBootstrap => "Hydrating...".to_string(),
            HydrationState::CatchUpDelta => "Catching up...".to_string(),
            HydrationState::LiveSteadyState => "Live".to_string(),
        }
    };

    let quack_badge = if let Some(uri) = store.quack_uri() {
        format!(" • 🦆 {uri}")
    } else {
        String::new()
    };

    let health_style = if offline {
        theme().health_warn
    } else {
        theme().health_ok
    };

    let project_label = widgets.filter_scope.project_scope.label();
    let sort_label = widgets.filter_scope.sort.label();

    let mut spans = Vec::with_capacity(20);
    let mut cur_x = area.x;
    let max_x = area.x.saturating_add(area.width);
    let y = area.y;
    let hitboxes = &mut widgets.hitboxes;

    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(
            format!(" 1: Unresolved ({unresolved_count}) "),
            tab_style(ListViewKind::Unresolved),
        ),
        Some(SlimEvent::SwitchListView(ListViewKind::Unresolved)),
    );
    push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw(" "), None);
    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(
            format!(" 2: Unhandled/High ({high_unhandled_count}) "),
            tab_style(ListViewKind::HighUnhandled),
        ),
        Some(SlimEvent::SwitchListView(ListViewKind::HighUnhandled)),
    );
    push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw(" "), None);
    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(
            format!(" 3: Incoming Feed ({feed_count}) "),
            tab_style(ListViewKind::IncomingFeed),
        ),
        Some(SlimEvent::SwitchListView(ListViewKind::IncomingFeed)),
    );
    push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw(" "), None);
    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(
            format!(" 4: All ({all_count}) "),
            tab_style(ListViewKind::AllIssues),
        ),
        Some(SlimEvent::SwitchListView(ListViewKind::AllIssues)),
    );
    push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw("  "), None);
    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(format!("[Org: {org_slug}]"), theme().org_badge),
        Some(SlimEvent::ManualRefresh),
    );
    push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw(" "), None);
    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(format!("[Project: {project_label}]"), theme().scope_pill),
        Some(SlimEvent::CycleProjectScope { reverse: false }),
    );
    push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw(" "), None);
    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(format!("[Sort: {sort_label}]"), theme().tag_key),
        Some(SlimEvent::CycleIssueSort),
    );

    if in_detail {
        push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw(" "), None);
        push_clickable_span(
            &mut spans,
            hitboxes,
            &mut cur_x,
            y,
            max_x,
            Span::styled(" [INSPECTING ISSUE] ", theme().active_tab),
            Some(SlimEvent::CloseIssueDetail),
        );
    }

    push_clickable_span(&mut spans, hitboxes, &mut cur_x, y, max_x, Span::raw("  "), None);
    push_clickable_span(
        &mut spans,
        hitboxes,
        &mut cur_x,
        y,
        max_x,
        Span::styled(format!("({sync_label}{quack_badge})"), health_style),
        Some(SlimEvent::ManualRefresh),
    );

    Paragraph::new(Line::from(spans)).render(area, buf);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        client::api::{parse_event_detail_json, parse_single_issue},
        config::{NotificationsSection, SlimConfig},
        domain::IssueDetailPage,
        event::PollBatchPayload,
        ui::widget::issue_detail_view::IssueDetailState,
    };

    #[test]
    fn test_single_page_list_and_multi_page_detail_rendering() {
        let cfg = SlimConfig::default();
        let mut store = SentryStore::new(PathBuf::from("/tmp/slim-test.duckdb"));
        let issue_val = serde_json::json!({
            "id": "7796091804",
            "shortId": "ACME-API-42",
            "title": "Oban.PerformError: App.ObanJobs.SyncWorker failed",
            "culprit": "lib/app/oban_jobs/cost_guard_job.ex",
            "permalink": "https://acme.sentry.io/issues/7796091804/",
            "level": "error",
            "status": "unresolved",
            "substatus": "ongoing",
            "priority": "high",
            "isUnhandled": true,
            "count": "5822",
            "userCount": 4,
            "firstSeen": "2026-10-16T13:00:05Z",
            "lastSeen": "2026-10-28T05:11:22Z",
            "project": {
                "id": "4509427291193344",
                "name": "elixir",
                "slug": "acme-api",
                "platform": "elixir"
            },
            "stats": {
                "24h": [[1793080800, 28], [1793084400, 30]]
            }
        });
        let issue = parse_single_issue(&issue_val).unwrap();
        let issue_id = issue.id.clone();

        store.apply_poll_batch(
            PollBatchPayload {
                projects: Vec::new(),
                issues: vec![issue],
                synced_at: Utc::now(),
                is_initial: true,
            },
            &NotificationsSection::default(),
            500,
            300,
        );

        let ev_val = serde_json::json!({
            "eventID": "d97a596029864efba244b0b69475a751",
            "dateCreated": "2026-10-28T05:11:22.778000Z",
            "title": "Oban.PerformError",
            "message": "",
            "culprit": "lib/app/oban_jobs/cost_guard_job.ex",
            "platform": "elixir",
            "tags": [{ "key": "environment", "value": "production" }],
            "context": { "oban_job_id": 376089 },
            "entries": [
                {
                    "type": "exception",
                    "data": {
                        "values": [{
                            "type": "Oban.PerformError",
                            "value": "token refresh failed",
                            "stacktrace": {
                                "frames": [{
                                    "filename": "lib/app/oban_jobs/cost_guard_job.ex",
                                    "function": "App.ObanJobs.CostGuardJob.perform/1",
                                    "lineNo": 111,
                                    "inApp": true
                                }]
                            }
                        }]
                    }
                }
            ]
        });
        let ev = parse_event_detail_json(&issue_id, &ev_val).unwrap();
        store.apply_issue_event(ev);

        let mut widgets = StatefulWidgets::new(&cfg);
        widgets.rebuild_derived(&store);

        let backend = TestBackend::new(120, 35);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Render main single-page list view
        terminal
            .draw(|f| {
                render_main_ui(f, &mut widgets, &store, "acme-org", false);
            })
            .unwrap();

        // Verify clicking the first issue row dispatches SelectAndOpenIssue
        let mut click_events = Vec::new();
        crate::input::process_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 10,
                row: 3, // header=0, border=1, table header=2, first row=3
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &mut widgets,
            &mut click_events,
        );
        assert_eq!(click_events.len(), 1);
        match &click_events[0] {
            SlimEvent::SelectAndOpenIssue {
                row_index,
                issue_id: clicked_id,
            } => {
                assert_eq!(*row_index, 0);
                assert_eq!(clicked_id, &issue_id);
            }
            other => panic!("unexpected event from row click: {other:?}"),
        }

        // 2. Open issue detail and render all 5 sub-pages
        widgets.open_issue_detail = Some(IssueDetailState::new(issue_id, false));
        for page in IssueDetailPage::ALL {
            widgets
                .open_issue_detail
                .as_mut()
                .unwrap()
                .switch_page(page);
            terminal
                .draw(|f| {
                    render_main_ui(f, &mut widgets, &store, "acme-org", false);
                })
                .unwrap();
        }

        // 3. Verify clicking the issue banner or URL on Overview dispatches OpenUrl
        widgets
            .open_issue_detail
            .as_mut()
            .unwrap()
            .switch_page(IssueDetailPage::Overview);
        terminal
            .draw(|f| {
                render_main_ui(f, &mut widgets, &store, "acme-org", false);
            })
            .unwrap();

        let mut banner_events = Vec::new();
        crate::input::process_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 10,
                row: 2, // Inside the sticky issue summary banner (rows 1..3)
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &mut widgets,
            &mut banner_events,
        );
        assert!(matches!(
            banner_events.first(),
            Some(SlimEvent::OpenUrl(url)) if url == "https://acme.sentry.io/issues/7796091804/"
        ));
    }
}
