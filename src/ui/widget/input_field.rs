use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::theme::theme;

pub struct FilterInputBar<'a> {
    pub value: &'a str,
    pub active: bool,
}

impl<'a> FilterInputBar<'a> {
    pub fn new(value: &'a str, active: bool) -> Self {
        Self { value, active }
    }
}

impl Widget for FilterInputBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let label_style = if self.active {
            theme().active_tab
        } else {
            theme().input_label
        };
        let cursor = if self.active { "█" } else { "" };
        let hint = if self.active {
            "  (Enter to lock filter, Esc to clear)"
        } else {
            "  (press '/' to edit filter, Esc to clear)"
        };

        let line = Line::from(vec![
            Span::styled(" / Filter: ", label_style),
            Span::raw(" "),
            Span::styled(format!("{}{}", self.value, cursor), theme().input_selected),
            Span::styled(hint, theme().input_description),
        ]);

        Paragraph::new(line).style(theme().input).render(area, buf);
    }
}
