use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Table, TableState,
    },
};

use crate::{
    domain::{ActivityEvent, ActivityKind, IssueStatus, ListViewKind, SentryIssue},
    gruvbox::Gruvbox,
    id::IssueId,
    theme::theme,
    ui::{format_compact_count, format_relative_time, render_sparkline_24h},
};

#[derive(Debug, Default, Clone)]
pub struct IssuesListState {
    pub rows: Vec<SentryIssue>,
    pub feed_rows: Vec<ActivityEvent>,
    pub table_state: TableState,
    pub show_sparklines: bool,
}

impl IssuesListState {
    pub fn len(&self, view: ListViewKind) -> usize {
        if view == ListViewKind::IncomingFeed {
            self.feed_rows.len()
        } else {
            self.rows.len()
        }
    }

    pub fn is_empty(&self, view: ListViewKind) -> bool {
        self.len(view) == 0
    }

    pub fn select_next(&mut self, view: ListViewKind) {
        let len = self.len(view);
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let next = match self.table_state.selected() {
            Some(i) => (i + 1).min(len - 1),
            None => 0,
        };
        self.table_state.select(Some(next));
    }

    pub fn select_prev(&mut self, view: ListViewKind) {
        let len = self.len(view);
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let prev = match self.table_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.table_state.select(Some(prev));
    }

    pub fn select_page_down(&mut self, view: ListViewKind, step: usize) {
        let len = self.len(view);
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let next = match self.table_state.selected() {
            Some(i) => (i + step).min(len - 1),
            None => 0,
        };
        self.table_state.select(Some(next));
    }

    pub fn select_page_up(&mut self, view: ListViewKind, step: usize) {
        let len = self.len(view);
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let prev = match self.table_state.selected() {
            Some(i) => i.saturating_sub(step),
            None => 0,
        };
        self.table_state.select(Some(prev));
    }

    pub fn select_first(&mut self, view: ListViewKind) {
        if self.is_empty(view) {
            self.table_state.select(None);
        } else {
            self.table_state.select(Some(0));
        }
    }

    pub fn select_last(&mut self, view: ListViewKind) {
        let len = self.len(view);
        if len == 0 {
            self.table_state.select(None);
        } else {
            self.table_state.select(Some(len - 1));
        }
    }

    pub fn selected_issue_id(&self, view: ListViewKind) -> Option<IssueId> {
        let idx = self.table_state.selected()?;
        if view == ListViewKind::IncomingFeed {
            self.feed_rows.get(idx).map(|e| e.issue_id.clone())
        } else {
            self.rows.get(idx).map(|i| i.id.clone())
        }
    }

    pub fn clamp_selection(&mut self, view: ListViewKind) {
        let len = self.len(view);
        if len == 0 {
            self.table_state.select(None);
        } else {
            let cur = self.table_state.selected().unwrap_or(0).min(len - 1);
            self.table_state.select(Some(cur));
        }
    }
}

pub struct IssuesTableWidget {
    pub view: ListViewKind,
    pub focused: bool,
}

impl IssuesTableWidget {
    pub fn new(view: ListViewKind, focused: bool) -> Self {
        Self { view, focused }
    }
}

impl StatefulWidget for IssuesTableWidget {
    type State = IssuesListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        state.clamp_selection(self.view);

        let border_style = if self.focused {
            theme().table_border_focused
        } else {
            theme().table_border
        };

        let total = state.len(self.view);
        let pos_label = match state.table_state.selected() {
            Some(idx) if total > 0 => format!(" {} — {}/{} ", self.view.label(), idx + 1, total),
            _ => format!(" {} (0) ", self.view.label()),
        };

        let block = Block::default()
            .title(Span::styled(pos_label, theme().border_title))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .style(theme().background);

        if self.view == ListViewKind::IncomingFeed {
            render_incoming_feed_table(area, buf, block, state);
        } else {
            render_issues_table(area, buf, block, state);
        }
    }
}

fn render_issues_table(
    area: Rect,
    buf: &mut Buffer,
    block: Block<'_>,
    state: &mut IssuesListState,
) {
    let header_style = Style::default()
        .fg(Gruvbox::OrangeBright.into())
        .bg(Gruvbox::Dark1.into())
        .add_modifier(Modifier::BOLD);

    let header = Row::new(vec![
        Cell::from("Short ID"),
        Cell::from("Level"),
        Cell::from("Status"),
        Cell::from("Prio"),
        Cell::from("Project"),
        Cell::from("Issue Title / Error Type"),
        Cell::from("24h Trend"),
        Cell::from("Events"),
        Cell::from("Users"),
        Cell::from("Seen"),
    ])
    .style(header_style)
    .height(1);

    let rows: Vec<Row> = state
        .rows
        .iter()
        .enumerate()
        .map(|(idx, iss)| {
            let row_bg = theme().table_row(idx);
            let unhandled_badge = if iss.is_unhandled { "!" } else { " " };
            let level_cell = Line::from(vec![
                Span::styled(
                    format!("{unhandled_badge}{:<5}", iss.level.label()),
                    theme().level_style(iss.level),
                ),
            ]);

            let status_text = match iss.status {
                IssueStatus::Unresolved => match iss.substatus.as_deref() {
                    Some("regressed") => "REGRESSED",
                    Some("escalating") => "ESCALATING",
                    Some("new") => "NEW",
                    _ => "UNRESOLVED",
                },
                IssueStatus::Resolved => "RESOLVED",
                IssueStatus::Ignored => "IGNORED",
            };
            let status_style = if iss.substatus.as_deref() == Some("regressed")
                || iss.substatus.as_deref() == Some("escalating")
            {
                theme().health_err
            } else {
                theme().status_style(iss.status)
            };

            let mut title_spans = Vec::new();
            title_spans.push(Span::styled(iss.title.to_string(), theme().issue_title));
            if !iss.culprit.is_empty() {
                title_spans.push(Span::styled(
                    format!("  ({})", iss.culprit),
                    theme().culprit,
                ));
            }

            let spark_str = if state.show_sparklines {
                let bars = render_sparkline_24h(&iss.stats_24h, 8);
                let c24 = format_compact_count(iss.events_24h());
                format!("{bars} {c24:>4}")
            } else {
                format_compact_count(iss.events_24h()).to_string()
            };

            Row::new(vec![
                Cell::from(Span::styled(iss.short_id.to_string(), theme().issue_id)),
                Cell::from(level_cell),
                Cell::from(Span::styled(status_text, status_style)),
                Cell::from(Span::styled(
                    iss.priority.label(),
                    theme().priority_style(iss.priority),
                )),
                Cell::from(Span::styled(
                    iss.project_slug.to_string(),
                    theme().project_name,
                )),
                Cell::from(Line::from(title_spans)),
                Cell::from(Span::styled(spark_str, theme().sparkline)),
                Cell::from(Span::styled(
                    format!("{:>6}", format_compact_count(iss.count)),
                    theme().time,
                )),
                Cell::from(Span::styled(
                    format!("{:>5}", format_compact_count(iss.user_count)),
                    theme().date,
                )),
                Cell::from(Span::styled(
                    format!("{:>6}", format_relative_time(iss.last_seen)),
                    theme().time,
                )),
            ])
            .style(row_bg)
        })
        .collect();

    let widths = [
        Constraint::Length(18),
        Constraint::Length(7),
        Constraint::Length(11),
        Constraint::Length(5),
        Constraint::Length(15),
        Constraint::Min(24),
        Constraint::Length(14),
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(7),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme().highlight_symbol)
        .highlight_symbol("▶ ");

    StatefulWidget::render(table, area, buf, &mut state.table_state);

    if state.rows.len() > (area.height.saturating_sub(3) as usize) {
        let mut sb_state = ScrollbarState::new(state.rows.len())
            .position(state.table_state.selected().unwrap_or(0));
        Scrollbar::new(ScrollbarOrientation::VerticalRight).render(area, buf, &mut sb_state);
    }
}

fn render_incoming_feed_table(
    area: Rect,
    buf: &mut Buffer,
    block: Block<'_>,
    state: &mut IssuesListState,
) {
    let header_style = Style::default()
        .fg(Gruvbox::OrangeBright.into())
        .bg(Gruvbox::Dark1.into())
        .add_modifier(Modifier::BOLD);

    let header = Row::new(vec![
        Cell::from("When"),
        Cell::from("Alert"),
        Cell::from("Project"),
        Cell::from("Short ID"),
        Cell::from("Activity Summary"),
        Cell::from("Issue Title"),
    ])
    .style(header_style)
    .height(1);

    let rows: Vec<Row> = state
        .feed_rows
        .iter()
        .enumerate()
        .map(|(idx, ev)| {
            let row_bg = theme().table_row(idx);
            let (badge_text, badge_style) = match &ev.kind {
                ActivityKind::NewIssue => ("NEW ISSUE", theme().health_err),
                ActivityKind::Regressed => ("REGRESSED", theme().health_err),
                ActivityKind::EventSpike { .. } => {
                    if ev.is_unhandled {
                        ("UNHANDLED", theme().health_err)
                    } else {
                        ("EVENT SPIKE", theme().health_warn)
                    }
                }
                ActivityKind::StatusChanged { .. } => ("STATUS", theme().health_ok),
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{:>6}", format_relative_time(ev.timestamp)),
                    theme().time,
                )),
                Cell::from(Span::styled(badge_text, badge_style)),
                Cell::from(Span::styled(
                    ev.project_slug.to_string(),
                    theme().project_name,
                )),
                Cell::from(Span::styled(ev.short_id.to_string(), theme().issue_id)),
                Cell::from(Span::styled(ev.summary.to_string(), theme().sparkline)),
                Cell::from(Span::styled(ev.issue_title.to_string(), theme().issue_title)),
            ])
            .style(row_bg)
        })
        .collect();

    let widths = [
        Constraint::Length(7),
        Constraint::Length(12),
        Constraint::Length(15),
        Constraint::Length(18),
        Constraint::Length(28),
        Constraint::Min(24),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme().highlight_symbol)
        .highlight_symbol("▶ ");

    StatefulWidget::render(table, area, buf, &mut state.table_state);

    if state.feed_rows.len() > (area.height.saturating_sub(3) as usize) {
        let mut sb_state = ScrollbarState::new(state.feed_rows.len())
            .position(state.table_state.selected().unwrap_or(0));
        Scrollbar::new(ScrollbarOrientation::VerticalRight).render(area, buf, &mut sb_state);
    }
}
