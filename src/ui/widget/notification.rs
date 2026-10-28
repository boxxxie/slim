use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use crate::{
    gruvbox::Gruvbox,
    notice_service::{Notice, NoticeLevel, NoticeMessage},
    theme::theme,
};

pub struct NotificationBanner<'a> {
    notice: &'a Notice,
}

impl<'a> NotificationBanner<'a> {
    pub fn new(notice: &'a Notice) -> Self {
        Self { notice }
    }

    pub fn banner_area(area: Rect) -> Rect {
        let width = (area.width.saturating_sub(6)).clamp(24, 78);
        let height = 3u16.min(area.height);
        let x = area.x + area.width.saturating_sub(width + 2);
        let y = area.y + 1;
        Rect::new(x, y, width, height)
    }
}

impl Widget for NotificationBanner<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rect = Self::banner_area(area);
        Clear.render(rect, buf);

        let (border_style, prefix) = match self.notice.level {
            NoticeLevel::Info => (
                Style::default()
                    .fg(Gruvbox::OrangeBright.into())
                    .bg(Gruvbox::Dark0Hard.into())
                    .add_modifier(Modifier::BOLD),
                " INFO ",
            ),
            NoticeLevel::Error => (
                Style::default()
                    .fg(Gruvbox::RedBright.into())
                    .bg(Gruvbox::Dark0Hard.into())
                    .add_modifier(Modifier::BOLD),
                " ERROR ",
            ),
        };

        let text = match &self.notice.message {
            NoticeMessage::GeneralMessage(s) => s.to_string(),
            NoticeMessage::InvalidSentryToken => {
                "Invalid Sentry token — press 'c' to configure or set SENTRY_AUTH_TOKEN".to_string()
            }
            NoticeMessage::ConfigError(s) => s.to_string(),
            NoticeMessage::SyncError(s) => format!("Sync error: {s}"),
        };

        let block = Block::default()
            .title(Span::styled(prefix, border_style))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .style(Style::default().bg(Gruvbox::Dark0Hard.into()));

        let inner = block.inner(rect);
        block.render(rect, buf);

        Paragraph::new(Line::from(Span::styled(text, theme().notification))).render(inner, buf);
    }
}
