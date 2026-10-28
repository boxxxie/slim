use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Widget},
};

#[derive(Debug, Clone)]
pub struct Window<'a> {
    title: Line<'a>,
    border_style: Style,
    border_type: BorderType,
    background: Style,
}

impl<'a> Window<'a> {
    pub fn new(title: impl Into<Line<'a>>) -> Self {
        Self {
            title: title.into(),
            border_style: Style::default(),
            border_type: BorderType::Rounded,
            background: Style::default(),
        }
    }

    pub fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    pub fn border_type(mut self, border_type: BorderType) -> Self {
        self.border_type = border_type;
        self
    }

    pub fn background(mut self, style: Style) -> Self {
        self.background = style;
        self
    }

    pub fn inner(&self, area: Rect) -> Rect {
        Block::default().borders(Borders::ALL).inner(area)
    }
}

impl Widget for Window<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        Block::default()
            .title(self.title)
            .borders(Borders::ALL)
            .border_type(self.border_type)
            .border_style(self.border_style)
            .style(self.background)
            .render(area, buf);
    }
}
