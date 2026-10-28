use compact_str::CompactString;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::{
    config::SlimConfig,
    theme::theme,
    ui::{fx::window::Window, popup::utility::CenteredShrink},
};

#[derive(Debug, Clone)]
pub struct ConfigPopup {
    pub config: SlimConfig,
    pub selected_field: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub token_edited: bool,
}

impl ConfigPopup {
    pub const FIELD_COUNT: usize = 6;

    pub fn new(config: SlimConfig) -> Self {
        Self {
            config,
            selected_field: 0,
            editing: false,
            edit_buffer: String::new(),
            token_edited: false,
        }
    }

    pub fn next_field(&mut self) {
        if !self.editing {
            self.selected_field = (self.selected_field + 1) % Self::FIELD_COUNT;
        }
    }

    pub fn prev_field(&mut self) {
        if !self.editing {
            self.selected_field =
                (self.selected_field + Self::FIELD_COUNT - 1) % Self::FIELD_COUNT;
        }
    }

    pub fn start_or_toggle_field(&mut self) {
        match self.selected_field {
            0 => {
                self.editing = true;
                self.edit_buffer = self.config.sentry.org.to_string();
            }
            1 => {
                self.editing = true;
                self.edit_buffer = self
                    .config
                    .sentry
                    .projects
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
            }
            2 => {
                self.editing = true;
                self.edit_buffer = self.config.polling.poll_seconds.to_string();
            }
            3 => {
                self.config.notifications.desktop_notifications_enabled =
                    !self.config.notifications.desktop_notifications_enabled;
            }
            4 => {
                self.config.effects.animations_enabled = !self.config.effects.animations_enabled;
            }
            5 => {
                self.editing = true;
                self.edit_buffer = self
                    .config
                    .sentry
                    .auth_token
                    .as_deref()
                    .unwrap_or_default()
                    .to_string();
            }
            _ => {}
        }
    }

    pub fn apply_edit(&mut self) {
        if !self.editing {
            return;
        }
        let trimmed = self.edit_buffer.trim();
        match self.selected_field {
            0 => {
                if !trimmed.is_empty() {
                    self.config.sentry.org = CompactString::from(trimmed);
                }
            }
            1 => {
                self.config.sentry.projects = trimmed
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(CompactString::from)
                    .collect();
            }
            2 => {
                if let Ok(secs) = trimmed.parse::<u64>() {
                    self.config.polling.poll_seconds = secs.max(15);
                }
            }
            5 => {
                self.token_edited = true;
                if trimmed.is_empty() {
                    self.config.sentry.auth_token = None;
                } else {
                    self.config.sentry.auth_token = Some(CompactString::from(trimmed));
                }
            }
            _ => {}
        }
        self.editing = false;
        self.edit_buffer.clear();
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    pub fn popup_area(terminal_area: Rect) -> Rect {
        terminal_area.inner_centered(74, 18)
    }

    pub fn render_in(&self, area: Rect, buf: &mut Buffer) {
        let popup_rect = Self::popup_area(area);
        let title = Line::from(vec![
            Span::styled(" Configuration ", theme().border.title),
            Span::styled(
                " (j/k select, Enter edit/toggle, Ctrl+s save, Esc close) ",
                theme().border_title,
            ),
        ]);

        let win = Window::new(title)
            .border_style(theme().border.config_border)
            .background(theme().background);
        let inner = win.inner(popup_rect);
        win.render(popup_rect, buf);

        let projects_val = if self.config.sentry.projects.is_empty() {
            "(all org projects)".to_string()
        } else {
            self.config
                .sentry
                .projects
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let token_val = match &self.config.sentry.auth_token {
            Some(t) if !t.is_empty() => "•••••••••••••••• (configured)".to_string(),
            _ => "(resolved from SENTRY_AUTH_TOKEN or ~/.sentryclirc)".to_string(),
        };

        let fields = [
            ("Sentry Org Slug", self.config.sentry.org.to_string()),
            ("Watched Projects", projects_val),
            (
                "Poll Interval (secs)",
                self.config.polling.poll_seconds.to_string(),
            ),
            (
                "Desktop Notifications",
                self.config
                    .notifications
                    .desktop_notifications_enabled
                    .to_string(),
            ),
            (
                "TachyonFX Animations",
                self.config.effects.animations_enabled.to_string(),
            ),
            ("Sentry Auth Token", token_val),
        ];

        let mut lines = Vec::with_capacity(fields.len() + 4);
        lines.push(Line::from(""));
        for (idx, (label, val)) in fields.iter().enumerate() {
            let selected = idx == self.selected_field;
            let prefix = if selected { " ▶ " } else { "   " };
            let display_val = if selected && self.editing {
                format!("{}█", self.edit_buffer)
            } else {
                val.clone()
            };
            let val_style = if selected {
                theme().active_tab
            } else {
                theme().tag_val
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{prefix}{label:<24}: "), theme().input_label),
                Span::styled(format!(" {display_val} "), val_style),
            ]));
            lines.push(Line::from(""));
        }

        Paragraph::new(lines).render(inner, buf);
    }
}
