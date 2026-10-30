use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::{
    gruvbox::Gruvbox,
    theme::theme,
    ui::{fx::window::Window, popup::utility::CenteredShrink},
};

#[derive(Debug, Default, Clone)]
pub struct HelpPopup {
    pub scroll: u16,
}

impl HelpPopup {
    pub fn new() -> Self {
        Self { scroll: 0 }
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_add(amount);
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    pub fn popup_area(terminal_area: Rect) -> Rect {
        terminal_area.percent_centered(78, 82)
    }

    pub fn render_in(&self, area: Rect, buf: &mut Buffer) {
        let popup_rect = Self::popup_area(area);
        let title = Line::from(vec![
            Span::styled(" slim ", theme().border.title),
            Span::styled(
                " Keyboard Shortcuts & Multi-Page Guide ",
                theme().border_title,
            ),
        ]);

        let win = Window::new(title)
            .border_style(theme().border.config_border)
            .background(theme().background);
        let inner = win.inner(popup_rect);
        win.render(popup_rect, buf);

        let key_style = Style::default()
            .fg(Gruvbox::OrangeBright.into())
            .add_modifier(Modifier::BOLD);
        let desc_style = theme().issue_title;
        let heading = theme().section_heading;

        let rows = vec![
            Line::from(Span::styled("Main Single-Page List Views", heading)),
            help_row("1", "Unresolved Issues view", key_style, desc_style),
            help_row(
                "2",
                "Unhandled & High-Priority Issues view",
                key_style,
                desc_style,
            ),
            help_row(
                "3",
                "Incoming Feed (live new issues, spikes & regressions)",
                key_style,
                desc_style,
            ),
            help_row(
                "4",
                "All Issues (including Resolved & Ignored)",
                key_style,
                desc_style,
            ),
            help_row(
                "p / P",
                "Cycle Project filter scope forward / backward",
                key_style,
                desc_style,
            ),
            help_row(
                "s",
                "Cycle sort order (Last Seen -> Events -> Users -> Priority -> First Seen)",
                key_style,
                desc_style,
            ),
            help_row(
                "/",
                "Filter issues by text (title, short ID, culprit, project, error type)",
                key_style,
                desc_style,
            ),
            help_row(
                "j / k, ↓ / ↑",
                "Move selection down / up",
                key_style,
                desc_style,
            ),
            help_row(
                "d / u, PgDn / PgUp",
                "Jump half-page / page down or up",
                key_style,
                desc_style,
            ),
            help_row("g / G", "Jump to first / last row", key_style, desc_style),
            help_row(
                "Enter / Left Click",
                "Open full-screen Multi-Page Issue Detail Inspector",
                key_style,
                desc_style,
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Full-Screen Multi-Page Issue Detail Inspector",
                heading,
            )),
            help_row(
                "1 .. 5",
                "Jump to page: 1=Overview & Tags, 2=Stacktrace, 3=Breadcrumbs, 4=Request/Contexts, 5=Raw JSON",
                key_style,
                desc_style,
            ),
            help_row(
                "Tab / ] / l / →",
                "Switch to next Issue Detail sub-page",
                key_style,
                desc_style,
            ),
            help_row(
                "Shift+Tab / [ / h / ←",
                "Switch to previous Issue Detail sub-page",
                key_style,
                desc_style,
            ),
            help_row(
                "j / k, ↓ / ↑",
                "Scroll current page down / up 1 line",
                key_style,
                desc_style,
            ),
            help_row(
                "d / u, Ctrl+d / Ctrl+u",
                "Scroll current page half-screen down / up",
                key_style,
                desc_style,
            ),
            help_row(
                "Space / PgDn / PgUp",
                "Scroll current page full-screen down / up",
                key_style,
                desc_style,
            ),
            help_row(
                "g / G, Home / End",
                "Jump to top / bottom of current page",
                key_style,
                desc_style,
            ),
            help_row(
                "a",
                "Toggle All Frames vs In-App Frames Only (on Page 2: Stacktrace)",
                key_style,
                desc_style,
            ),
            help_row(
                "v",
                "Toggle expanding frame local variables & source context (on Page 2: Stacktrace)",
                key_style,
                desc_style,
            ),
            help_row(
                "Esc / Backspace",
                "Return to the Issues List view",
                key_style,
                desc_style,
            ),
            Line::from(""),
            Line::from(Span::styled("Triage Actions, Clipboard & Global", heading)),
            help_row(
                "R",
                "Resolve selected/current Sentry issue",
                key_style,
                desc_style,
            ),
            help_row(
                "I",
                "Ignore (archive) selected/current Sentry issue",
                key_style,
                desc_style,
            ),
            help_row(
                "U",
                "Unresolve selected/current Sentry issue",
                key_style,
                desc_style,
            ),
            help_row(
                "y",
                "Copy issue Short ID (e.g. ACME-API-42) to clipboard",
                key_style,
                desc_style,
            ),
            help_row(
                "Y",
                "Copy full Markdown issue summary + stacktrace to clipboard",
                key_style,
                desc_style,
            ),
            help_row(
                "o / w",
                "Open issue permalink in default web browser",
                key_style,
                desc_style,
            ),
            help_row(
                "r",
                "Trigger immediate manual poll / refresh current issue event",
                key_style,
                desc_style,
            ),
            help_row("n", "Recall last notification banner", key_style, desc_style),
            help_row("c", "Open Configuration popup", key_style, desc_style),
            help_row("?", "Toggle this Help popup", key_style, desc_style),
            help_row("q", "Quit slim (or close current view)", key_style, desc_style),
        ];

        Paragraph::new(rows)
            .scroll((self.scroll, 0))
            .render(inner, buf);
    }
}

fn help_row(
    key: &'static str,
    desc: &'static str,
    key_style: Style,
    desc_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<24}", key), key_style),
        Span::styled(desc.to_string(), desc_style),
    ])
}
