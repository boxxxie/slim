use chrono::{DateTime, Local, TimeZone, Utc};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Widget, Wrap,
    },
};

use crate::{
    domain::{
        ActivityEvent, IssueDetailPage, SentryEventDetail, SentryIssue,
    },
    event::SlimEvent,
    gruvbox::Gruvbox,
    id::IssueId,
    stores::SentryStore,
    theme::theme,
    ui::{format_compact_count, format_relative_time, render_sparkline_24h},
};

#[derive(Debug, Clone)]
pub struct IssueDetailState {
    pub issue_id: IssueId,
    pub active_page: IssueDetailPage,
    pub page_scroll: [u16; 5],
    pub in_app_frames_only: bool,
    pub expand_frame_vars: bool,
    pub loading_event: bool,
    pub last_viewport_height: u16,
}

impl IssueDetailState {
    pub fn new(issue_id: IssueId, loading_event: bool) -> Self {
        Self {
            issue_id,
            active_page: IssueDetailPage::Overview,
            page_scroll: [0; 5],
            in_app_frames_only: false,
            expand_frame_vars: true,
            loading_event,
            last_viewport_height: 20,
        }
    }

    pub fn current_scroll(&self) -> u16 {
        self.page_scroll[self.active_page.index()]
    }

    pub fn set_scroll(&mut self, val: u16) {
        self.page_scroll[self.active_page.index()] = val;
    }

    pub fn scroll_down(&mut self, amount: u16) {
        let idx = self.active_page.index();
        self.page_scroll[idx] = self.page_scroll[idx].saturating_add(amount);
    }

    pub fn scroll_up(&mut self, amount: u16) {
        let idx = self.active_page.index();
        self.page_scroll[idx] = self.page_scroll[idx].saturating_sub(amount);
    }

    pub fn switch_page(&mut self, page: IssueDetailPage) {
        self.active_page = page;
    }
}

pub struct IssueDetailView<'a> {
    store: &'a SentryStore,
    state: &'a mut IssueDetailState,
    hitboxes: &'a mut Vec<(Rect, SlimEvent)>,
}

impl<'a> IssueDetailView<'a> {
    pub fn new(
        store: &'a SentryStore,
        state: &'a mut IssueDetailState,
        hitboxes: &'a mut Vec<(Rect, SlimEvent)>,
    ) -> Self {
        Self {
            store,
            state,
            hitboxes,
        }
    }

    pub fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(issue) = self.store.issues.get(&self.state.issue_id) else {
            Paragraph::new("Issue not found in local store.")
                .style(theme().health_err)
                .render(area, buf);
            return;
        };
        let event = self.store.events_by_issue.get(&self.state.issue_id);

        if area.height < 6 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Sticky Issue Summary Banner
                Constraint::Length(1), // Sub-page Tab Bar (1..5)
                Constraint::Min(3),    // Active Page Content
            ])
            .split(area);

        render_issue_banner(
            chunks[0],
            buf,
            issue,
            event,
            self.state.loading_event,
            self.hitboxes,
        );
        render_subpage_tabs(chunks[1], buf, self.state, event, self.hitboxes);

        let body_area = chunks[2];
        let inner_height = body_area.height.saturating_sub(2).max(1);
        self.state.last_viewport_height = inner_height;

        let lines = match self.state.active_page {
            IssueDetailPage::Overview => build_overview_page(issue, event, self.state.loading_event),
            IssueDetailPage::Stacktrace => build_stacktrace_page(
                issue,
                event,
                self.state.in_app_frames_only,
                self.state.expand_frame_vars,
                self.state.loading_event,
            ),
            IssueDetailPage::Breadcrumbs => {
                let issue_acts: Vec<&ActivityEvent> = self
                    .store
                    .activity_events
                    .iter()
                    .filter(|a| a.issue_id == issue.id)
                    .collect();
                build_breadcrumbs_page(event, &issue_acts, self.state.loading_event)
            }
            IssueDetailPage::RequestContexts => {
                build_request_contexts_page(issue, event, self.state.loading_event)
            }
            IssueDetailPage::RawJson => build_raw_json_page(issue, event),
        };

        let total_lines = lines.len().max(1);
        let vh = inner_height as usize;
        let max_scroll = total_lines.saturating_sub(vh) as u16;
        let clamped_scroll = self.state.current_scroll().min(max_scroll);
        self.state.set_scroll(clamped_scroll);

        let cur_screen_page = (clamped_scroll as usize / vh) + 1;
        let total_screen_pages = total_lines.div_ceil(vh).max(1);
        let start_line = (clamped_scroll as usize) + 1;
        let end_line = (clamped_scroll as usize + vh).min(total_lines);

        let page_title = Line::from(vec![
            Span::styled(
                format!(" {} ", self.state.active_page.title()),
                theme().border.title,
            ),
            Span::styled(
                format!(
                    "  Screen Page {cur_screen_page}/{total_screen_pages} • Lines {start_line}–{end_line} of {total_lines}  (Tab / [ ] / 1-5 switch tab, j/k/PgDn/PgUp scroll, Esc back) "
                ),
                theme().border_title,
            ),
        ]);

        let block = Block::default()
            .title(page_title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme().table_border_focused)
            .style(theme().background);

        let inner = block.inner(body_area);
        block.render(body_area, buf);

        if self.state.active_page == IssueDetailPage::Overview && !issue.permalink.is_empty() {
            let url_line_idx = if issue.culprit.is_empty() { 1usize } else { 2usize };
            let scroll_idx = clamped_scroll as usize;
            if url_line_idx >= scroll_idx && url_line_idx < scroll_idx + (inner.height as usize) {
                let url_y = inner.y + (url_line_idx - scroll_idx) as u16;
                self.hitboxes.push((
                    Rect {
                        x: inner.x,
                        y: url_y,
                        width: inner.width,
                        height: 1,
                    },
                    SlimEvent::OpenUrl(issue.permalink.clone()),
                ));
            }
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((clamped_scroll, 0))
            .render(inner, buf);

        if total_lines > vh {
            let mut sb_state =
                ScrollbarState::new(total_lines).position(clamped_scroll as usize);
            Scrollbar::new(ScrollbarOrientation::VerticalRight).render(
                body_area,
                buf,
                &mut sb_state,
            );
        }
    }
}

fn render_issue_banner(
    area: Rect,
    buf: &mut Buffer,
    issue: &SentryIssue,
    event: Option<&SentryEventDetail>,
    loading_event: bool,
    hitboxes: &mut Vec<(Rect, SlimEvent)>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme().table_border)
        .style(Style::default().bg(Gruvbox::Dark0Hard.into()));
    let inner = block.inner(area);
    block.render(area, buf);

    if !issue.permalink.is_empty() && area.width > 0 && area.height > 0 {
        hitboxes.push((area, SlimEvent::OpenUrl(issue.permalink.clone())));
    }

    let unhandled_str = if issue.is_unhandled {
        " • UNHANDLED "
    } else {
        ""
    };
    let sub_str = issue
        .substatus
        .as_deref()
        .map(|s| format!(" ({s})"))
        .unwrap_or_default();
    let env_str = event
        .and_then(|e| e.environment.as_deref())
        .unwrap_or("default");
    let event_status = if loading_event && event.is_none() {
        " [Loading latest event...]"
    } else if let Some(ev) = event {
        &format!(" [Event: {}]", &ev.event_id.as_str()[..ev.event_id.as_str().len().min(12)])
    } else {
        ""
    };

    let line = Line::from(vec![
        Span::styled(format!(" {} ", issue.short_id), theme().issue_id),
        Span::raw(" "),
        Span::styled(
            format!(" {}{} ", issue.level.label(), unhandled_str),
            theme().level_style(issue.level),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{}{}", issue.status.label(), sub_str),
            theme().status_style(issue.status),
        ),
        Span::raw("  "),
        Span::styled("Prio: ", theme().input_label),
        Span::styled(
            issue.priority.label(),
            theme().priority_style(issue.priority),
        ),
        Span::raw("  "),
        Span::styled("Proj: ", theme().input_label),
        Span::styled(issue.project_slug.as_str(), theme().project_name),
        Span::raw("  "),
        Span::styled("Env: ", theme().input_label),
        Span::styled(env_str, theme().scope_pill),
        Span::raw("  "),
        Span::styled("Events: ", theme().input_label),
        Span::styled(format_compact_count(issue.count).to_string(), theme().time),
        Span::raw("  "),
        Span::styled("Users: ", theme().input_label),
        Span::styled(
            format_compact_count(issue.user_count).to_string(),
            theme().date,
        ),
        Span::raw("  "),
        Span::styled("Seen: ", theme().input_label),
        Span::styled(
            format_relative_time(issue.last_seen).to_string(),
            theme().time,
        ),
        Span::styled(event_status.to_string(), theme().input_description),
    ]);

    Paragraph::new(line).render(inner, buf);
}

fn render_subpage_tabs(
    area: Rect,
    buf: &mut Buffer,
    state: &IssueDetailState,
    event: Option<&SentryEventDetail>,
    hitboxes: &mut Vec<(Rect, SlimEvent)>,
) {
    let mut spans = Vec::with_capacity(16);
    let mut cur_x = area.x;
    let max_x = area.x.saturating_add(area.width);
    let y = area.y;

    for page in IssueDetailPage::ALL {
        let is_active = state.active_page == page;
        let style = if is_active {
            theme().active_tab
        } else {
            theme().inactive_tab
        };
        let label = match page {
            IssueDetailPage::Overview => {
                let tag_cnt = event.map(|e| e.tags.len()).unwrap_or(0);
                format!(" 1: Overview & Tags ({tag_cnt}) ")
            }
            IssueDetailPage::Stacktrace => {
                let frame_cnt = event.map(|e| e.total_frames()).unwrap_or(0);
                format!(" 2: Exception & Stacktrace ({frame_cnt} frames) ")
            }
            IssueDetailPage::Breadcrumbs => {
                let bc_cnt = event.map(|e| e.breadcrumbs.len()).unwrap_or(0);
                format!(" 3: Breadcrumbs ({bc_cnt}) ")
            }
            IssueDetailPage::RequestContexts => {
                let ctx_cnt = event
                    .map(|e| {
                        e.custom_context.len()
                            + e.runtime_contexts.len()
                            + usize::from(e.request.is_some())
                    })
                    .unwrap_or(0);
                format!(" 4: Request & Contexts ({ctx_cnt}) ")
            }
            IssueDetailPage::RawJson => " 5: Raw Event JSON ".to_string(),
        };
        let span = Span::styled(label, style);
        let w = span.width() as u16;
        let clamped_w = w.min(max_x.saturating_sub(cur_x));
        if clamped_w > 0 {
            hitboxes.push((
                Rect {
                    x: cur_x,
                    y,
                    width: clamped_w,
                    height: 1,
                },
                SlimEvent::SwitchDetailPage(page),
            ));
        }
        cur_x = cur_x.saturating_add(w);
        spans.push(span);

        let sep = Span::raw("  ");
        cur_x = cur_x.saturating_add(sep.width() as u16);
        spans.push(sep);
    }
    Paragraph::new(Line::from(spans)).render(area, buf);
}

fn build_overview_page(
    issue: &SentryIssue,
    event: Option<&SentryEventDetail>,
    loading_event: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("Title:   ", theme().input_label),
        Span::styled(
            issue.title.to_string(),
            theme().issue_title.add_modifier(Modifier::BOLD),
        ),
    ]));
    if !issue.culprit.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Culprit: ", theme().input_label),
            Span::styled(issue.culprit.to_string(), theme().culprit),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("URL:     ", theme().input_label),
        Span::styled(issue.permalink.to_string(), theme().date),
    ]));
    lines.push(Line::from(""));

    // Core Properties Table
    lines.push(Line::from(Span::styled(
        "━━ Issue & Event Properties ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));

    let first_local: DateTime<Local> = issue.first_seen.with_timezone(&Local);
    let last_local: DateTime<Local> = issue.last_seen.with_timezone(&Local);

    lines.push(Line::from(vec![
        Span::styled("  Issue ID:      ", theme().tag_key),
        Span::styled(format!("{:<22}", issue.id), theme().tag_val),
        Span::styled("Short ID:      ", theme().tag_key),
        Span::styled(issue.short_id.to_string(), theme().issue_id),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Project:       ", theme().tag_key),
        Span::styled(
            format!("{:<22}", format!("{} ({})", issue.project_slug, issue.project_name)),
            theme().project_name,
        ),
        Span::styled("Platform:      ", theme().tag_key),
        Span::styled(
            issue.platform.as_deref().unwrap_or("unknown").to_string(),
            theme().tag_val,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Status:        ", theme().tag_key),
        Span::styled(
            format!(
                "{:<22}",
                format!(
                    "{} ({})",
                    issue.status.label(),
                    issue.substatus.as_deref().unwrap_or("active")
                )
            ),
            theme().status_style(issue.status),
        ),
        Span::styled("Unhandled:     ", theme().tag_key),
        Span::styled(
            if issue.is_unhandled { "YES (!)" } else { "no" },
            if issue.is_unhandled {
                theme().health_err
            } else {
                theme().tag_val
            },
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Total Events:  ", theme().tag_key),
        Span::styled(format!("{:<22}", issue.count), theme().time),
        Span::styled("Events (24h):  ", theme().tag_key),
        Span::styled(issue.events_24h().to_string(), theme().sparkline),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Users Affected:", theme().tag_key),
        Span::styled(format!(" {:<21}", issue.user_count), theme().tag_val),
        Span::styled("Assigned To:   ", theme().tag_key),
        Span::styled(
            issue
                .assigned_to
                .as_deref()
                .unwrap_or("Unassigned")
                .to_string(),
            theme().tag_val,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  First Seen:    ", theme().tag_key),
        Span::styled(
            format!(
                "{:<22}",
                format!(
                    "{} ({})",
                    first_local.format("%Y-%m-%d %H:%M"),
                    format_relative_time(issue.first_seen)
                )
            ),
            theme().date,
        ),
        Span::styled("Last Seen:     ", theme().tag_key),
        Span::styled(
            format!(
                "{} ({})",
                last_local.format("%Y-%m-%d %H:%M"),
                format_relative_time(issue.last_seen)
            ),
            theme().time,
        ),
    ]));

    if let Some(ev) = event {
        lines.push(Line::from(vec![
            Span::styled("  Latest Event:  ", theme().tag_key),
            Span::styled(format!("{:<22}", ev.event_id), theme().tag_val),
            Span::styled("Release:       ", theme().tag_key),
            Span::styled(
                ev.release.as_deref().unwrap_or("none").to_string(),
                theme().tag_val,
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  SDK:           ", theme().tag_key),
            Span::styled(
                format!(
                    "{:<22}",
                    ev.sdk_info
                        .as_deref()
                        .or(issue.sdk_name.as_deref())
                        .unwrap_or("unknown")
                ),
                theme().tag_val,
            ),
            Span::styled("Payload Size:  ", theme().tag_key),
            Span::styled(
                ev.size_bytes
                    .map(|b| format!("{b} bytes"))
                    .unwrap_or_else(|| "—".to_string()),
                theme().date,
            ),
        ]));
    }

    // Error / Message Summary
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "━━ Error / Message Summary ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));
    if let Some(ref et) = issue.error_type {
        lines.push(Line::from(vec![
            Span::styled(format!("  {et}: "), theme().health_err),
            Span::styled(
                issue.error_value.as_deref().unwrap_or_default().to_string(),
                theme().issue_title,
            ),
        ]));
    } else if let Some(ev) = event {
        if let Some(ref fmt_msg) = ev.formatted_message {
            for l in fmt_msg.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {l}"),
                    theme().issue_title,
                )));
            }
        } else if !ev.message.is_empty() {
            for l in ev.message.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {l}"),
                    theme().issue_title,
                )));
            }
        } else {
            lines.push(Line::from(Span::styled(
                format!("  {}", issue.title),
                theme().issue_title,
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {}", issue.title),
            theme().issue_title,
        )));
    }

    // 24h Sparkline & Hourly Histogram
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "━━ 24-Hour Event Activity ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));
    let spark_24 = render_sparkline_24h(&issue.stats_24h, 24);
    lines.push(Line::from(vec![
        Span::styled("  24h Sparkline: ", theme().tag_key),
        Span::styled(spark_24, theme().sparkline),
        Span::styled(
            format!("   ({} events in last 24h)", issue.events_24h()),
            theme().time,
        ),
    ]));
    if !issue.stats_24h.is_empty() {
        let mut bucket_spans = vec![Span::raw("  Hourly: ")];
        for &(ts, cnt) in issue.stats_24h.iter().rev().take(8) {
            let hr = Utc
                .timestamp_opt(ts, 0)
                .single()
                .map(|dt| dt.with_timezone(&Local).format("%H:00").to_string())
                .unwrap_or_else(|| "?".to_string());
            bucket_spans.push(Span::styled(format!("{hr}="), theme().date));
            bucket_spans.push(Span::styled(format!("{cnt}  "), theme().time));
        }
        lines.push(Line::from(bucket_spans));
    }

    // Event Tags
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "━━ Event Tags ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));
    match event {
        Some(ev) if !ev.tags.is_empty() => {
            for tag in &ev.tags {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<22}", tag.key), theme().tag_key),
                    Span::styled(tag.value.to_string(), theme().tag_val),
                ]));
            }
        }
        Some(_) => {
            lines.push(Line::from(Span::styled(
                "  (No tags attached to latest event)",
                theme().input_description,
            )));
        }
        None => {
            let msg = if loading_event {
                "  Loading latest event tags from Sentry..."
            } else {
                "  Latest event not cached yet (press 'r' to fetch)"
            };
            lines.push(Line::from(Span::styled(msg, theme().input_description)));
        }
    }

    lines
}

fn build_stacktrace_page(
    issue: &SentryIssue,
    event: Option<&SentryEventDetail>,
    in_app_only: bool,
    expand_vars: bool,
    loading_event: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let filter_label = if in_app_only {
        "IN-APP FRAMES ONLY (press 'a' for All)"
    } else {
        "ALL FRAMES (press 'a' for In-App Only)"
    };
    let vars_label = if expand_vars {
        "VARS & CONTEXT EXPANDED (press 'v' to Collapse)"
    } else {
        "VARS & CONTEXT COLLAPSED (press 'v' to Expand)"
    };

    lines.push(Line::from(vec![
        Span::styled("Filter: ", theme().input_label),
        Span::styled(filter_label, theme().scope_pill),
        Span::raw("    "),
        Span::styled("Detail: ", theme().input_label),
        Span::styled(vars_label, theme().org_badge),
    ]));
    lines.push(Line::from(""));

    let Some(ev) = event else {
        let msg = if loading_event {
            "Loading exception & stacktrace from Sentry..."
        } else {
            "Latest event not loaded yet. Press 'r' to fetch from Sentry."
        };
        lines.push(Line::from(Span::styled(msg, theme().input_description)));
        return lines;
    };

    if ev.exceptions.is_empty() {
        lines.push(Line::from(Span::styled(
            "━━ Message Event (No Exception Stacktrace) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
            theme().section_heading,
        )));
        if let Some(ref fmt_msg) = ev.formatted_message {
            for l in fmt_msg.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {l}"),
                    theme().issue_title,
                )));
            }
        } else if !ev.message.is_empty() {
            for l in ev.message.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {l}"),
                    theme().issue_title,
                )));
            }
        } else {
            lines.push(Line::from(Span::styled(
                format!("  {}", issue.title),
                theme().issue_title,
            )));
        }
        return lines;
    }

    for (exc_idx, exc) in ev.exceptions.iter().enumerate() {
        let mech_str = match (&exc.mechanism_type, exc.handled) {
            (Some(m), Some(h)) => format!(" [mechanism: {m}, handled: {h}]"),
            (Some(m), None) => format!(" [mechanism: {m}]"),
            (None, Some(h)) => format!(" [handled: {h}]"),
            (None, None) => String::new(),
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("━━ Exception #{}: {} ", exc_idx + 1, exc.exc_type),
                theme().health_err,
            ),
            Span::styled(mech_str, theme().date),
        ]));

        if !exc.value.is_empty() {
            for l in exc.value.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {l}"),
                    theme().issue_title,
                )));
            }
        }
        lines.push(Line::from(""));

        if exc.frames.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (No stacktrace frames recorded for this exception)",
                theme().input_description,
            )));
            lines.push(Line::from(""));
            continue;
        }

        // Display most recent call first (reverse order, like Sentry web UI default)
        let mut shown = 0usize;
        for (idx, frame) in exc.frames.iter().enumerate().rev() {
            if in_app_only && !frame.in_app {
                continue;
            }
            shown += 1;
            let badge = if frame.in_app { "[APP]" } else { "[SYS]" };
            let badge_style = if frame.in_app {
                theme().frame_in_app
            } else {
                theme().frame_system
            };
            let func = frame.function.as_deref().unwrap_or("<anonymous>");
            let loc = frame.display_location();

            lines.push(Line::from(vec![
                Span::styled(format!("  #{:<3} ", idx + 1), theme().date),
                Span::styled(format!("{badge} "), badge_style),
                Span::styled(func.to_string(), theme().frame_func),
                Span::raw("  at "),
                Span::styled(loc, theme().frame_loc),
            ]));

            if expand_vars {
                if !frame.context_lines.is_empty() {
                    for (ln, code) in &frame.context_lines {
                        let is_target = frame.line_no == Some(*ln);
                        let prefix = if is_target { "    ▶ " } else { "      " };
                        let style = if is_target {
                            theme().health_warn
                        } else {
                            theme().date
                        };
                        lines.push(Line::from(Span::styled(
                            format!("{prefix}{ln:>4} │ {code}"),
                            style,
                        )));
                    }
                }
                if !frame.vars.is_empty() {
                    for (k, v) in &frame.vars {
                        lines.push(Line::from(vec![
                            Span::raw("        "),
                            Span::styled(format!("{k} = "), theme().tag_key),
                            Span::styled(v.clone(), theme().tag_val),
                        ]));
                    }
                }
            }
        }

        if shown == 0 && in_app_only {
            lines.push(Line::from(Span::styled(
                format!(
                    "  All {} frames are system/library frames — press 'a' to show all frames.",
                    exc.frames.len()
                ),
                theme().health_warn,
            )));
        }
        lines.push(Line::from(""));
    }

    lines
}

fn build_breadcrumbs_page(
    event: Option<&SentryEventDetail>,
    issue_activities: &[&ActivityEvent],
    loading_event: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        "━━ Event Breadcrumbs ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));

    match event {
        Some(ev) if !ev.breadcrumbs.is_empty() => {
            for (idx, b) in ev.breadcrumbs.iter().enumerate().rev() {
                let ts = b.timestamp.as_deref().unwrap_or("—");
                lines.push(Line::from(vec![
                    Span::styled(format!("  #{:<3} ", idx + 1), theme().date),
                    Span::styled(format!("[{}] ", b.level), theme().scope_pill),
                    Span::styled(
                        format!("{}/{} ", b.crumb_type, b.category),
                        theme().tag_key,
                    ),
                    Span::styled(format!("({ts})"), theme().date),
                ]));
                if !b.message.is_empty() {
                    for l in b.message.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("       {l}"),
                            theme().issue_title,
                        )));
                    }
                }
                for (k, v) in &b.data_fields {
                    lines.push(Line::from(vec![
                        Span::raw("       "),
                        Span::styled(format!("{k}: "), theme().tag_key),
                        Span::styled(v.clone(), theme().tag_val),
                    ]));
                }
                lines.push(Line::from(""));
            }
        }
        Some(_) => {
            lines.push(Line::from(Span::styled(
                "  (No breadcrumbs recorded on the latest event)",
                theme().input_description,
            )));
        }
        None => {
            let msg = if loading_event {
                "  Loading breadcrumbs from Sentry..."
            } else {
                "  Latest event not loaded yet (press 'r' to fetch)."
            };
            lines.push(Line::from(Span::styled(msg, theme().input_description)));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "━━ Local Monitor Timeline ({} recorded events) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
            issue_activities.len()
        ),
        theme().section_heading,
    )));

    if issue_activities.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (No local activity transitions recorded yet)",
            theme().input_description,
        )));
    } else {
        for act in issue_activities {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:>6} ago  ", format_relative_time(act.timestamp)),
                    theme().date,
                ),
                Span::styled(act.summary.to_string(), theme().sparkline),
            ]));
        }
    }

    lines
}

fn build_request_contexts_page(
    issue: &SentryIssue,
    event: Option<&SentryEventDetail>,
    loading_event: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let Some(ev) = event else {
        let msg = if loading_event {
            "Loading request & context data from Sentry..."
        } else {
            "Latest event not loaded yet. Press 'r' to fetch from Sentry."
        };
        lines.push(Line::from(Span::styled(msg, theme().input_description)));
        return lines;
    };

    // 1. Custom / Extra Context (e.g. Oban worker, attempt, job_id)
    lines.push(Line::from(Span::styled(
        "━━ Custom / Extra Context (event.context) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));
    if ev.custom_context.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (No custom extra context fields)",
            theme().input_description,
        )));
    } else {
        for (k, v) in &ev.custom_context {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<24}", k), theme().tag_key),
                Span::styled(v.clone(), theme().tag_val),
            ]));
        }
    }
    lines.push(Line::from(""));

    // 2. HTTP Request
    lines.push(Line::from(Span::styled(
        "━━ HTTP Request ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));
    match &ev.request {
        Some(req)
            if req.method.is_some()
                || req.url.is_some()
                || !req.headers.is_empty()
                || !req.env.is_empty()
                || req.data_body.is_some() =>
        {
            if req.method.is_some() || req.url.is_some() {
                lines.push(Line::from(vec![
                    Span::styled("  Endpoint:     ", theme().tag_key),
                    Span::styled(
                        format!(
                            "{} {}",
                            req.method.as_deref().unwrap_or("REQ"),
                            req.url.as_deref().unwrap_or("<no url>")
                        ),
                        theme().org_badge,
                    ),
                ]));
            }
            if let Some(ref ct) = req.inferred_content_type {
                lines.push(Line::from(vec![
                    Span::styled("  Content-Type: ", theme().tag_key),
                    Span::styled(ct.to_string(), theme().tag_val),
                ]));
            }
            if !req.query.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  Query Parameters:",
                    theme().input_label,
                )));
                for (k, v) in &req.query {
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {:<22}", k), theme().tag_key),
                        Span::styled(v.clone(), theme().tag_val),
                    ]));
                }
            }
            if !req.headers.is_empty() {
                lines.push(Line::from(Span::styled("  Headers:", theme().input_label)));
                for (k, v) in &req.headers {
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {:<26}", k), theme().tag_key),
                        Span::styled(v.clone(), theme().tag_val),
                    ]));
                }
            }
            if !req.env.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  Server Env:",
                    theme().input_label,
                )));
                for (k, v) in &req.env {
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {:<22}", k), theme().tag_key),
                        Span::styled(v.clone(), theme().tag_val),
                    ]));
                }
            }
            if let Some(ref body) = req.data_body {
                lines.push(Line::from(Span::styled("  Body:", theme().input_label)));
                for l in body.lines().take(50) {
                    lines.push(Line::from(Span::styled(
                        format!("    {l}"),
                        theme().issue_title,
                    )));
                }
            }
        }
        _ => {
            lines.push(Line::from(Span::styled(
                "  (No HTTP request metadata attached to this event)",
                theme().input_description,
            )));
        }
    }
    lines.push(Line::from(""));

    // 3. User Context
    lines.push(Line::from(Span::styled(
        "━━ User Context ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));
    if let Some(ref u) = ev.user {
        for (label, val) in [
            ("ID", u.id.as_deref()),
            ("Username", u.username.as_deref()),
            ("Email", u.email.as_deref()),
            ("Name", u.name.as_deref()),
            ("IP Address", u.ip_address.as_deref()),
            ("Geo", u.geo.as_deref()),
        ] {
            if let Some(v) = val {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<24}", label), theme().tag_key),
                    Span::styled(v.to_string(), theme().tag_val),
                ]));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  (No user identity attached)",
            theme().input_description,
        )));
    }
    lines.push(Line::from(""));

    // 4. Structured Runtime Contexts (OS, Runtime, Trace, etc.)
    lines.push(Line::from(Span::styled(
        "━━ Runtime, OS & Trace Contexts ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        theme().section_heading,
    )));
    if ev.runtime_contexts.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (No runtime contexts recorded)",
            theme().input_description,
        )));
    } else {
        for (ctx_name, fields) in &ev.runtime_contexts {
            lines.push(Line::from(Span::styled(
                format!("  [{ctx_name}]"),
                theme().org_badge,
            )));
            for (k, v) in fields {
                lines.push(Line::from(vec![
                    Span::styled(format!("    {:<22}", k), theme().tag_key),
                    Span::styled(v.clone(), theme().tag_val),
                ]));
            }
        }
    }

    if !ev.packages.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "━━ Loaded Packages ({}) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
                ev.packages.len()
            ),
            theme().section_heading,
        )));
        for (pkg, ver) in ev.packages.iter().take(60) {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<32}", pkg), theme().tag_key),
                Span::styled(ver.to_string(), theme().date),
            ]));
        }
    }

    let _ = issue;
    lines
}

fn build_raw_json_page(
    issue: &SentryIssue,
    event: Option<&SentryEventDetail>,
) -> Vec<Line<'static>> {
    let raw = if let Some(ev) = event {
        ev.raw_json.as_str()
    } else if let Some(ref iss_json) = issue.raw_json {
        iss_json.as_str()
    } else {
        "{}"
    };

    raw.lines()
        .enumerate()
        .map(|(idx, line)| {
            Line::from(vec![
                Span::styled(format!("{:>5} │ ", idx + 1), theme().date),
                Span::styled(line.to_string(), theme().issue_title),
            ])
        })
        .collect()
}
