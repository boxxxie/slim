use once_cell::sync::Lazy;
use ratatui::style::{Modifier, Style};

use crate::{
    domain::{IssueLevel, IssuePriority, IssueStatus},
    gruvbox::Gruvbox,
};

pub struct Theme {
    pub org_badge: Style,
    pub project_name: Style,
    pub project_description: Style,
    pub issue_id: Style,
    pub issue_title: Style,
    pub culprit: Style,
    pub tag_key: Style,
    pub tag_val: Style,
    pub date: Style,
    pub time: Style,
    pub sparkline: Style,
    pub highlight_symbol: Style,
    pub table_border: Style,
    pub table_border_focused: Style,
    pub table_row_a: Style,
    pub table_row_b: Style,
    pub background: Style,
    pub border_title: Style,
    pub active_tab: Style,
    pub inactive_tab: Style,
    pub scope_pill: Style,
    pub health_ok: Style,
    pub health_warn: Style,
    pub health_err: Style,
    pub notification: Style,
    pub input: Style,
    pub input_selected: Style,
    pub input_description: Style,
    pub input_label: Style,
    pub configuration_error: Style,
    pub frame_in_app: Style,
    pub frame_system: Style,
    pub frame_func: Style,
    pub frame_loc: Style,
    pub section_heading: Style,
    pub border: ThemeBorder,
}

pub struct ThemeBorder {
    pub config_border: Style,
    pub title: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self::new()
    }
}

impl Theme {
    pub fn new() -> Self {
        Self {
            org_badge: Style::default()
                .fg(Gruvbox::OrangeBright.into())
                .add_modifier(Modifier::BOLD),
            project_name: Style::default()
                .fg(Gruvbox::AquaBright.into())
                .add_modifier(Modifier::BOLD),
            project_description: Style::default()
                .fg(Gruvbox::Light4.into())
                .add_modifier(Modifier::ITALIC),
            issue_id: Style::default()
                .fg(Gruvbox::YellowBright.into())
                .add_modifier(Modifier::BOLD),
            issue_title: Style::default().fg(Gruvbox::Light1.into()),
            culprit: Style::default()
                .fg(Gruvbox::PurpleBright.into())
                .add_modifier(Modifier::ITALIC),
            tag_key: Style::default()
                .fg(Gruvbox::BlueBright.into())
                .add_modifier(Modifier::BOLD),
            tag_val: Style::default().fg(Gruvbox::Light2.into()),
            date: Style::default().fg(Gruvbox::Gray244.into()),
            time: Style::default().fg(Gruvbox::Light2.into()),
            sparkline: Style::default().fg(Gruvbox::OrangeBright.into()),
            highlight_symbol: Style::default().bg(Gruvbox::Dark2.into()),
            table_border: Style::default()
                .fg(Gruvbox::Dark4.into())
                .bg(Gruvbox::Dark0.into()),
            table_border_focused: Style::default()
                .fg(Gruvbox::Orange.into())
                .bg(Gruvbox::Dark0.into()),
            table_row_a: Style::default().bg(Gruvbox::Dark0Hard.into()),
            table_row_b: Style::default().bg(Gruvbox::Dark0.into()),
            background: Style::default().bg(Gruvbox::Dark0.into()),
            border_title: Style::default()
                .fg(Gruvbox::Light2.into())
                .add_modifier(Modifier::BOLD),
            active_tab: Style::default()
                .fg(Gruvbox::Dark0Hard.into())
                .bg(Gruvbox::OrangeBright.into())
                .add_modifier(Modifier::BOLD),
            inactive_tab: Style::default().fg(Gruvbox::Light4.into()),
            scope_pill: Style::default()
                .fg(Gruvbox::AquaBright.into())
                .add_modifier(Modifier::BOLD),
            health_ok: Style::default()
                .fg(Gruvbox::GreenBright.into())
                .add_modifier(Modifier::BOLD),
            health_warn: Style::default()
                .fg(Gruvbox::YellowBright.into())
                .add_modifier(Modifier::BOLD),
            health_err: Style::default()
                .fg(Gruvbox::RedBright.into())
                .add_modifier(Modifier::BOLD),
            notification: Style::default()
                .bg(Gruvbox::Dark0.into())
                .fg(Gruvbox::Orange.into())
                .add_modifier(Modifier::BOLD),
            input: Style::default()
                .fg(Gruvbox::Light2.into())
                .bg(Gruvbox::Dark0Hard.into())
                .add_modifier(Modifier::BOLD),
            input_selected: Style::default()
                .fg(Gruvbox::Light0Soft.into())
                .bg(Gruvbox::Dark0Hard.into())
                .add_modifier(Modifier::BOLD),
            input_label: Style::default()
                .fg(Gruvbox::Orange.into())
                .add_modifier(Modifier::BOLD),
            input_description: Style::default()
                .fg(Gruvbox::Gray244.into())
                .add_modifier(Modifier::ITALIC),
            configuration_error: Style::default()
                .fg(Gruvbox::YellowBright.into())
                .add_modifier(Modifier::BOLD),
            frame_in_app: Style::default()
                .fg(Gruvbox::GreenBright.into())
                .add_modifier(Modifier::BOLD),
            frame_system: Style::default().fg(Gruvbox::Gray244.into()),
            frame_func: Style::default()
                .fg(Gruvbox::YellowBright.into())
                .add_modifier(Modifier::BOLD),
            frame_loc: Style::default().fg(Gruvbox::AquaBright.into()),
            section_heading: Style::default()
                .fg(Gruvbox::OrangeBright.into())
                .add_modifier(Modifier::BOLD),
            border: ThemeBorder {
                title: Style::default()
                    .bg(Gruvbox::Orange.into())
                    .fg(Gruvbox::Dark0.into())
                    .add_modifier(Modifier::BOLD),
                config_border: Style::default()
                    .fg(Gruvbox::OrangeBright.into())
                    .bg(Gruvbox::Dark0.into()),
            },
        }
    }

    pub fn table_row(&self, idx: usize) -> Style {
        match idx % 2 {
            0 => self.table_row_a,
            _ => self.table_row_b,
        }
    }

    pub fn level_style(&self, level: IssueLevel) -> Style {
        match level {
            IssueLevel::Fatal => Style::default()
                .fg(Gruvbox::Dark0Hard.into())
                .bg(Gruvbox::RedBright.into())
                .add_modifier(Modifier::BOLD),
            IssueLevel::Error => Style::default()
                .fg(Gruvbox::RedBright.into())
                .add_modifier(Modifier::BOLD),
            IssueLevel::Warning => Style::default()
                .fg(Gruvbox::YellowBright.into())
                .add_modifier(Modifier::BOLD),
            IssueLevel::Info => Style::default().fg(Gruvbox::BlueBright.into()),
            IssueLevel::Debug => Style::default().fg(Gruvbox::Gray244.into()),
        }
    }

    pub fn status_style(&self, status: IssueStatus) -> Style {
        match status {
            IssueStatus::Unresolved => Style::default()
                .fg(Gruvbox::OrangeBright.into())
                .add_modifier(Modifier::BOLD),
            IssueStatus::Resolved => Style::default()
                .fg(Gruvbox::GreenBright.into())
                .add_modifier(Modifier::BOLD),
            IssueStatus::Ignored => Style::default().fg(Gruvbox::Gray244.into()),
        }
    }

    pub fn priority_style(&self, priority: IssuePriority) -> Style {
        match priority {
            IssuePriority::High => Style::default()
                .fg(Gruvbox::RedBright.into())
                .add_modifier(Modifier::BOLD),
            IssuePriority::Medium => Style::default().fg(Gruvbox::YellowBright.into()),
            IssuePriority::Low => Style::default().fg(Gruvbox::BlueBright.into()),
            IssuePriority::None => Style::default().fg(Gruvbox::Gray244.into()),
        }
    }
}

static THEME: Lazy<Theme> = Lazy::new(Theme::new);
pub(crate) fn theme() -> &'static Theme {
    &THEME
}
