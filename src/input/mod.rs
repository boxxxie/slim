use compact_str::CompactString;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::{
    domain::{IssueDetailPage, IssueStatus, ListViewKind},
    event::SlimEvent,
    ui::stateful_widgets::{ModalState, StatefulWidgets},
};

pub fn process_key_event(
    key: KeyEvent,
    widgets: &mut StatefulWidgets,
    out: &mut Vec<SlimEvent>,
) {
    // 1. Ctrl+C always shuts down
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        out.push(SlimEvent::Shutdown);
        return;
    }

    // 2. Active modal (Config or Help)
    if let Some(modal) = widgets.modal.clone() {
        match modal {
            ModalState::Help(help_rc) => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                        out.push(SlimEvent::HelpClose);
                    }
                    KeyCode::Char('j') | KeyCode::Down => help_rc.borrow_mut().scroll_down(1),
                    KeyCode::Char('k') | KeyCode::Up => help_rc.borrow_mut().scroll_up(1),
                    KeyCode::PageDown | KeyCode::Char('d') => help_rc.borrow_mut().scroll_down(10),
                    KeyCode::PageUp | KeyCode::Char('u') => help_rc.borrow_mut().scroll_up(10),
                    _ => {}
                }
                return;
            }
            ModalState::Config(cfg_rc) => {
                let mut cfg = cfg_rc.borrow_mut();
                if cfg.editing {
                    match key.code {
                        KeyCode::Enter => cfg.apply_edit(),
                        KeyCode::Esc => cfg.cancel_edit(),
                        KeyCode::Backspace => {
                            cfg.edit_buffer.pop();
                        }
                        KeyCode::Char(c) => {
                            cfg.edit_buffer.push(c);
                        }
                        _ => {}
                    }
                    return;
                }

                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                    out.push(SlimEvent::ConfigApply {
                        config: cfg.config.clone(),
                        persist_token: cfg.token_edited,
                    });
                    return;
                }

                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => out.push(SlimEvent::ConfigClose),
                    KeyCode::Char('j') | KeyCode::Down => cfg.next_field(),
                    KeyCode::Char('k') | KeyCode::Up => cfg.prev_field(),
                    KeyCode::Enter | KeyCode::Char(' ') => cfg.start_or_toggle_field(),
                    _ => {}
                }
                return;
            }
        }
    }

    // 3. Active filter input bar
    if widgets.filtering {
        match key.code {
            KeyCode::Enter => {
                let q = CompactString::from(widgets.filter_input.trim());
                out.push(SlimEvent::FilterInputApply(q));
            }
            KeyCode::Esc => {
                out.push(SlimEvent::FilterInputClear);
            }
            KeyCode::Backspace => {
                out.push(SlimEvent::FilterInputBackspace);
            }
            KeyCode::Char(c) => {
                out.push(SlimEvent::FilterInputChar(CompactString::from(
                    c.to_string(),
                )));
            }
            _ => {}
        }
        return;
    }

    // 4. Multi-Page Issue Detail View
    if let Some(ref detail) = widgets.open_issue_detail {
        let vh = detail.last_viewport_height.max(4);
        let half_vh = (vh / 2).max(1);
        let full_vh = vh.saturating_sub(2).max(1);

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('d') => {
                    out.push(SlimEvent::DetailScrollDown(half_vh));
                    return;
                }
                KeyCode::Char('u') => {
                    out.push(SlimEvent::DetailScrollUp(half_vh));
                    return;
                }
                KeyCode::Char('f') => {
                    out.push(SlimEvent::DetailScrollDown(full_vh));
                    return;
                }
                KeyCode::Char('b') => {
                    out.push(SlimEvent::DetailScrollUp(full_vh));
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') => {
                out.push(SlimEvent::CloseIssueDetail);
            }
            KeyCode::Char('1') => out.push(SlimEvent::SwitchDetailPage(IssueDetailPage::Overview)),
            KeyCode::Char('2') => {
                out.push(SlimEvent::SwitchDetailPage(IssueDetailPage::Stacktrace))
            }
            KeyCode::Char('3') => {
                out.push(SlimEvent::SwitchDetailPage(IssueDetailPage::Breadcrumbs))
            }
            KeyCode::Char('4') => out.push(SlimEvent::SwitchDetailPage(
                IssueDetailPage::RequestContexts,
            )),
            KeyCode::Char('5') => out.push(SlimEvent::SwitchDetailPage(IssueDetailPage::RawJson)),
            KeyCode::Tab | KeyCode::Char(']') | KeyCode::Char('l') | KeyCode::Right => {
                out.push(SlimEvent::NextDetailPage);
            }
            KeyCode::BackTab | KeyCode::Char('[') | KeyCode::Char('h') | KeyCode::Left => {
                out.push(SlimEvent::PrevDetailPage);
            }
            KeyCode::Char('j') | KeyCode::Down => out.push(SlimEvent::DetailScrollDown(1)),
            KeyCode::Char('k') | KeyCode::Up => out.push(SlimEvent::DetailScrollUp(1)),
            KeyCode::Char('d') => out.push(SlimEvent::DetailScrollDown(half_vh)),
            KeyCode::Char('u') => out.push(SlimEvent::DetailScrollUp(half_vh)),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                out.push(SlimEvent::DetailScrollDown(full_vh))
            }
            KeyCode::PageUp => out.push(SlimEvent::DetailScrollUp(full_vh)),
            KeyCode::Char('g') | KeyCode::Home => out.push(SlimEvent::DetailScrollTop),
            KeyCode::Char('G') | KeyCode::End => out.push(SlimEvent::DetailScrollBottom),
            KeyCode::Char('a') => out.push(SlimEvent::ToggleInAppFramesOnly),
            KeyCode::Char('v') => out.push(SlimEvent::ToggleExpandFrameVars),
            KeyCode::Char('R') => out.push(SlimEvent::MutateSelectedStatus(IssueStatus::Resolved)),
            KeyCode::Char('I') => out.push(SlimEvent::MutateSelectedStatus(IssueStatus::Ignored)),
            KeyCode::Char('U') => {
                out.push(SlimEvent::MutateSelectedStatus(IssueStatus::Unresolved))
            }
            KeyCode::Char('y') => out.push(SlimEvent::CopySelectedShortId),
            KeyCode::Char('Y') => out.push(SlimEvent::CopySelectedMarkdownSummary),
            KeyCode::Char('o') | KeyCode::Char('w') => out.push(SlimEvent::OpenSelectedUrl),
            KeyCode::Char('r') => out.push(SlimEvent::ManualRefresh),
            KeyCode::Char('n') => out.push(SlimEvent::NotificationRecallLast),
            KeyCode::Char('c') => out.push(SlimEvent::ConfigOpen),
            KeyCode::Char('?') => out.push(SlimEvent::HelpOpen),
            _ => {}
        }
        return;
    }

    // 5. Main Single-Page Issues List View
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('d') => {
                out.push(SlimEvent::CursorPageDown);
                return;
            }
            KeyCode::Char('u') => {
                out.push(SlimEvent::CursorPageUp);
                return;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('q') => out.push(SlimEvent::Shutdown),
        KeyCode::Esc => out.push(SlimEvent::EscapeOrBack),
        KeyCode::Char('1') => out.push(SlimEvent::SwitchListView(ListViewKind::Unresolved)),
        KeyCode::Char('2') => out.push(SlimEvent::SwitchListView(ListViewKind::HighUnhandled)),
        KeyCode::Char('3') => out.push(SlimEvent::SwitchListView(ListViewKind::IncomingFeed)),
        KeyCode::Char('4') => out.push(SlimEvent::SwitchListView(ListViewKind::AllIssues)),
        KeyCode::Char('p') => out.push(SlimEvent::CycleProjectScope { reverse: false }),
        KeyCode::Char('P') => out.push(SlimEvent::CycleProjectScope { reverse: true }),
        KeyCode::Char('s') => out.push(SlimEvent::CycleIssueSort),
        KeyCode::Char('/') => out.push(SlimEvent::FilterInputStart),
        KeyCode::Char('j') | KeyCode::Down => out.push(SlimEvent::CursorNext),
        KeyCode::Char('k') | KeyCode::Up => out.push(SlimEvent::CursorPrevious),
        KeyCode::PageDown | KeyCode::Char('d') => out.push(SlimEvent::CursorPageDown),
        KeyCode::PageUp | KeyCode::Char('u') => out.push(SlimEvent::CursorPageUp),
        KeyCode::Char('g') | KeyCode::Home => out.push(SlimEvent::CursorFirst),
        KeyCode::Char('G') | KeyCode::End => out.push(SlimEvent::CursorLast),
        KeyCode::Enter => out.push(SlimEvent::OpenSelectedIssue),
        KeyCode::Char('R') => out.push(SlimEvent::MutateSelectedStatus(IssueStatus::Resolved)),
        KeyCode::Char('I') => out.push(SlimEvent::MutateSelectedStatus(IssueStatus::Ignored)),
        KeyCode::Char('U') => out.push(SlimEvent::MutateSelectedStatus(IssueStatus::Unresolved)),
        KeyCode::Char('y') => out.push(SlimEvent::CopySelectedShortId),
        KeyCode::Char('Y') => out.push(SlimEvent::CopySelectedMarkdownSummary),
        KeyCode::Char('o') | KeyCode::Char('w') => out.push(SlimEvent::OpenSelectedUrl),
        KeyCode::Char('r') => out.push(SlimEvent::ManualRefresh),
        KeyCode::Char('n') => out.push(SlimEvent::NotificationRecallLast),
        KeyCode::Char('c') => out.push(SlimEvent::ConfigOpen),
        KeyCode::Char('?') => out.push(SlimEvent::HelpOpen),
        _ => {}
    }
}

pub fn process_mouse_event(
    mouse: MouseEvent,
    widgets: &mut StatefulWidgets,
    out: &mut Vec<SlimEvent>,
) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if widgets.modal.is_some() {
                return;
            }
            let x = mouse.column;
            let y = mouse.row;
            for (rect, ev) in widgets.hitboxes.iter().rev() {
                if x >= rect.x
                    && x < rect.x.saturating_add(rect.width)
                    && y >= rect.y
                    && y < rect.y.saturating_add(rect.height)
                {
                    out.push(ev.clone());
                    return;
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if widgets.open_issue_detail.is_some() {
                out.push(SlimEvent::DetailScrollDown(3));
            } else {
                out.push(SlimEvent::CursorNext);
            }
        }
        MouseEventKind::ScrollUp => {
            if widgets.open_issue_detail.is_some() {
                out.push(SlimEvent::DetailScrollUp(3));
            } else {
                out.push(SlimEvent::CursorPrevious);
            }
        }
        _ => {}
    }
}
