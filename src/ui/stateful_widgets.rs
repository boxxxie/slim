use std::{cell::RefCell, rc::Rc};

use compact_str::CompactString;

use crate::{
    config::SlimConfig,
    domain::{
        ActivityEvent, IssueLevel, IssueSort, IssueStatus, ListViewKind, SentryIssue,
    },
    id::{IssueId, ProjectScope},
    notice_service::Notice,
    stores::SentryStore,
    ui::{
        popup::{config_popup::ConfigPopup, help_popup::HelpPopup},
        widget::{
            issue_detail_view::IssueDetailState,
            issues_table::IssuesListState,
            shortcuts::{Shortcut, Shortcuts},
        },
    },
};

#[derive(Debug, Clone)]
pub enum ModalState {
    Config(Rc<RefCell<ConfigPopup>>),
    Help(Rc<RefCell<HelpPopup>>),
}

#[derive(Debug, Default, Clone)]
pub struct FilterScope {
    pub project_scope: ProjectScope,
    pub sort: IssueSort,
    pub search_query: CompactString,
}

pub struct StatefulWidgets {
    pub active_list_view: ListViewKind,
    pub filter_scope: FilterScope,
    pub filtering: bool,
    pub filter_input: String,
    pub list_state: IssuesListState,
    pub open_issue_detail: Option<IssueDetailState>,
    pub modal: Option<ModalState>,
    pub active_notice: Option<Notice>,
    pub last_synced_generation: u64,
}

impl StatefulWidgets {
    pub fn new(config: &SlimConfig) -> Self {
        let mut list_state = IssuesListState::default();
        list_state.show_sparklines = config.views.show_sparklines;
        Self {
            active_list_view: config.views.default_view,
            filter_scope: FilterScope::default(),
            filtering: false,
            filter_input: String::new(),
            list_state,
            open_issue_detail: None,
            modal: None,
            active_notice: None,
            last_synced_generation: 0,
        }
    }

    pub fn is_in_detail_view(&self) -> bool {
        self.open_issue_detail.is_some()
    }

    pub fn current_target_issue_id(&self) -> Option<IssueId> {
        if let Some(ref detail) = self.open_issue_detail {
            Some(detail.issue_id.clone())
        } else {
            self.list_state.selected_issue_id(self.active_list_view)
        }
    }

    pub fn rebuild_derived(&mut self, store: &SentryStore) {
        let prev_selected_id = self.list_state.selected_issue_id(self.active_list_view);
        let q = self.filter_scope.search_query.trim().to_ascii_lowercase();

        // 1. Rebuild filtered issues
        let mut issues: Vec<SentryIssue> = store
            .issues
            .values()
            .filter(|iss| {
                if let ProjectScope::Project(ref slug) = self.filter_scope.project_scope {
                    if !iss.project_slug.eq_ignore_ascii_case(slug.as_str()) {
                        return false;
                    }
                }
                match self.active_list_view {
                    ListViewKind::Unresolved => {
                        if iss.status != IssueStatus::Unresolved {
                            return false;
                        }
                    }
                    ListViewKind::HighUnhandled => {
                        if iss.status != IssueStatus::Unresolved {
                            return false;
                        }
                        let high_or_unhandled = iss.is_unhandled
                            || iss.level == IssueLevel::Fatal
                            || iss.priority == crate::domain::IssuePriority::High
                            || iss.substatus.as_deref() == Some("regressed")
                            || iss.substatus.as_deref() == Some("escalating");
                        if !high_or_unhandled {
                            return false;
                        }
                    }
                    ListViewKind::IncomingFeed | ListViewKind::AllIssues => {}
                }
                if !q.is_empty() {
                    let matched = iss.title.to_ascii_lowercase().contains(&q)
                        || iss.short_id.to_ascii_lowercase().contains(&q)
                        || iss.culprit.to_ascii_lowercase().contains(&q)
                        || iss.project_slug.to_ascii_lowercase().contains(&q)
                        || iss
                            .error_type
                            .as_deref()
                            .unwrap_or_default()
                            .to_ascii_lowercase()
                            .contains(&q);
                    if !matched {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        match self.filter_scope.sort {
            IssueSort::LastSeen => issues.sort_by(|a, b| b.last_seen.cmp(&a.last_seen)),
            IssueSort::Events => issues.sort_by(|a, b| {
                b.count
                    .cmp(&a.count)
                    .then_with(|| b.last_seen.cmp(&a.last_seen))
            }),
            IssueSort::Users => issues.sort_by(|a, b| {
                b.user_count
                    .cmp(&a.user_count)
                    .then_with(|| b.count.cmp(&a.count))
            }),
            IssueSort::Priority => issues.sort_by(|a, b| {
                b.priority
                    .rank()
                    .cmp(&a.priority.rank())
                    .then_with(|| b.level.severity_rank().cmp(&a.level.severity_rank()))
                    .then_with(|| b.last_seen.cmp(&a.last_seen))
            }),
            IssueSort::FirstSeen => issues.sort_by(|a, b| b.first_seen.cmp(&a.first_seen)),
        }

        // 2. Rebuild filtered incoming feed events
        let feed_rows: Vec<ActivityEvent> = store
            .activity_events
            .iter()
            .filter(|ev| {
                if let ProjectScope::Project(ref slug) = self.filter_scope.project_scope {
                    if !ev.project_slug.eq_ignore_ascii_case(slug.as_str()) {
                        return false;
                    }
                }
                if !q.is_empty() {
                    let matched = ev.issue_title.to_ascii_lowercase().contains(&q)
                        || ev.short_id.to_ascii_lowercase().contains(&q)
                        || ev.summary.to_ascii_lowercase().contains(&q)
                        || ev.project_slug.to_ascii_lowercase().contains(&q);
                    if !matched {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        self.list_state.rows = issues;
        self.list_state.feed_rows = feed_rows;

        if let Some(prev_id) = prev_selected_id {
            let pos = if self.active_list_view == ListViewKind::IncomingFeed {
                self.list_state
                    .feed_rows
                    .iter()
                    .position(|e| e.issue_id == prev_id)
            } else {
                self.list_state.rows.iter().position(|i| i.id == prev_id)
            };
            if let Some(idx) = pos {
                self.list_state.table_state.select(Some(idx));
            } else {
                self.list_state.clamp_selection(self.active_list_view);
            }
        } else {
            self.list_state.clamp_selection(self.active_list_view);
        }

        self.last_synced_generation = store.generation;
    }

    pub fn active_shortcuts(&self) -> Shortcuts {
        if self.open_issue_detail.is_some() {
            Shortcuts::new(vec![
                Shortcut::new("1-5/Tab/[]", "Sub-Page"),
                Shortcut::new("j/k/d/u/PgDn", "Scroll"),
                Shortcut::new("a", "In-App Frames"),
                Shortcut::new("v", "Vars/Context"),
                Shortcut::new("R/I/U", "Resolve/Ignore/Unres"),
                Shortcut::new("y/Y", "Copy ID/Markdown"),
                Shortcut::new("o", "Browser"),
                Shortcut::new("Esc", "Back to List"),
                Shortcut::new("?", "Help"),
            ])
        } else {
            Shortcuts::new(vec![
                Shortcut::new("1-4", "Views"),
                Shortcut::new("Enter", "Inspect Issue"),
                Shortcut::new("p", "Project"),
                Shortcut::new("s", "Sort"),
                Shortcut::new("/", "Filter"),
                Shortcut::new("R/I/U", "Resolve/Ignore/Unres"),
                Shortcut::new("y/Y", "Copy ID/Markdown"),
                Shortcut::new("o", "Browser"),
                Shortcut::new("r", "Refresh"),
                Shortcut::new("?", "Help"),
                Shortcut::new("q", "Quit"),
            ])
        }
    }
}
