use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::gruvbox::Gruvbox;

#[derive(Debug, Clone)]
pub struct Shortcut {
    pub key: &'static str,
    pub action: &'static str,
}

impl Shortcut {
    pub const fn new(key: &'static str, action: &'static str) -> Self {
        Self { key, action }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Shortcuts {
    pub items: Vec<Shortcut>,
}

impl Shortcuts {
    pub fn new(items: Vec<Shortcut>) -> Self {
        Self { items }
    }

    pub fn as_line(&self) -> Line<'static> {
        let key_style = Style::default()
            .fg(Gruvbox::OrangeBright.into())
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Gruvbox::Light3.into());
        let sep_style = Style::default().fg(Gruvbox::Dark4.into());

        let mut spans: Vec<Span<'static>> = Vec::with_capacity(self.items.len() * 4);
        for (idx, item) in self.items.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::styled(" │ ", sep_style));
            }
            spans.push(Span::styled(format!("[{}] ", item.key), key_style));
            spans.push(Span::styled(item.action.to_string(), desc_style));
        }
        Line::from(spans)
    }
}
