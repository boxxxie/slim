use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Paragraph, StatefulWidget, Widget},
};

use crate::{
    domain::{HydrationState, IssueLevel, IssueStatus, ListViewKind},
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
    }

    let focused = widgets.modal.is_none();

    if let Some(ref mut detail_state) = widgets.open_issue_detail {
        IssueDetailView::new(store, detail_state).render(body_area, buf);
    } else {
        IssuesTableWidget::new(widgets.active_list_view, focused).render(
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
    }
}

fn render_top_chrome(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    widgets: &StatefulWidgets,
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

    let tab_style = |v: ListViewKind| {
        if !widgets.is_in_detail_view() && widgets.active_list_view == v {
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

    let mut spans = vec![
        Span::styled(
            format!(" 1: Unresolved ({unresolved_count}) "),
            tab_style(ListViewKind::Unresolved),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" 2: Unhandled/High ({high_unhandled_count}) "),
            tab_style(ListViewKind::HighUnhandled),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" 3: Incoming Feed ({feed_count}) "),
            tab_style(ListViewKind::IncomingFeed),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" 4: All ({all_count}) "),
            tab_style(ListViewKind::AllIssues),
        ),
        Span::raw("  "),
        Span::styled(format!("[Org: {org_slug}]"), theme().org_badge),
        Span::raw(" "),
        Span::styled(
            format!("[Project: {}]", widgets.filter_scope.project_scope.label()),
            theme().scope_pill,
        ),
        Span::raw(" "),
        Span::styled(
            format!("[Sort: {}]", widgets.filter_scope.sort.label()),
            theme().tag_key,
        ),
    ];

    if widgets.is_in_detail_view() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(" [INSPECTING ISSUE] ", theme().active_tab));
    }

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("({sync_label}{quack_badge})"),
        health_style,
    ));

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
    }
}
