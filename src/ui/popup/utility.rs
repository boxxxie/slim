use ratatui::layout::Rect;

pub trait CenteredShrink {
    fn inner_centered(&self, width: u16, height: u16) -> Rect;
    fn percent_centered(&self, percent_x: u16, percent_y: u16) -> Rect;
}

impl CenteredShrink for Rect {
    fn inner_centered(&self, width: u16, height: u16) -> Rect {
        let w = width.min(self.width);
        let h = height.min(self.height);
        let x = self.x + (self.width.saturating_sub(w)) / 2;
        let y = self.y + (self.height.saturating_sub(h)) / 2;
        Rect::new(x, y, w, h)
    }

    fn percent_centered(&self, percent_x: u16, percent_y: u16) -> Rect {
        let w = (u32::from(self.width) * u32::from(percent_x.min(100)) / 100) as u16;
        let h = (u32::from(self.height) * u32::from(percent_y.min(100)) / 100) as u16;
        self.inner_centered(w.max(20), h.max(8))
    }
}
