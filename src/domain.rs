use chrono::{DateTime, Utc};
use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use crate::id::{ActivityId, EventId, IssueId, ProjectId};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListViewKind {
    #[default]
    Unresolved,
    HighUnhandled,
    IncomingFeed,
    AllIssues,
}

impl ListViewKind {
    pub fn label(&self) -> &'static str {
        match self {
            ListViewKind::Unresolved => "Unresolved",
            ListViewKind::HighUnhandled => "Unhandled & High",
            ListViewKind::IncomingFeed => "Incoming Feed",
            ListViewKind::AllIssues => "All Issues",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IssueDetailPage {
    #[default]
    Overview,
    Stacktrace,
    Breadcrumbs,
    RequestContexts,
    RawJson,
}

impl IssueDetailPage {
    pub const ALL: [IssueDetailPage; 5] = [
        IssueDetailPage::Overview,
        IssueDetailPage::Stacktrace,
        IssueDetailPage::Breadcrumbs,
        IssueDetailPage::RequestContexts,
        IssueDetailPage::RawJson,
    ];

    pub fn index(&self) -> usize {
        match self {
            IssueDetailPage::Overview => 0,
            IssueDetailPage::Stacktrace => 1,
            IssueDetailPage::Breadcrumbs => 2,
            IssueDetailPage::RequestContexts => 3,
            IssueDetailPage::RawJson => 4,
        }
    }

    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            1 => Some(Self::Overview),
            2 => Some(Self::Stacktrace),
            3 => Some(Self::Breadcrumbs),
            4 => Some(Self::RequestContexts),
            5 => Some(Self::RawJson),
            _ => None,
        }
    }

    pub fn next(&self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(&self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn title(&self) -> &'static str {
        match self {
            IssueDetailPage::Overview => "1: Overview & Tags",
            IssueDetailPage::Stacktrace => "2: Exception & Stacktrace",
            IssueDetailPage::Breadcrumbs => "3: Breadcrumbs & Timeline",
            IssueDetailPage::RequestContexts => "4: Request & Contexts",
            IssueDetailPage::RawJson => "5: Raw Event JSON",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueLevel {
    Fatal,
    #[default]
    Error,
    Warning,
    Info,
    Debug,
}

impl IssueLevel {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "fatal" | "critical" => Self::Fatal,
            "error" | "err" => Self::Error,
            "warning" | "warn" => Self::Warning,
            "info" | "log" => Self::Info,
            "debug" => Self::Debug,
            _ => Self::Error,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            IssueLevel::Fatal => "FATAL",
            IssueLevel::Error => "ERROR",
            IssueLevel::Warning => "WARN",
            IssueLevel::Info => "INFO",
            IssueLevel::Debug => "DEBUG",
        }
    }

    pub fn severity_rank(&self) -> u8 {
        match self {
            IssueLevel::Fatal => 5,
            IssueLevel::Error => 4,
            IssueLevel::Warning => 3,
            IssueLevel::Info => 2,
            IssueLevel::Debug => 1,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueStatus {
    #[default]
    Unresolved,
    Resolved,
    Ignored,
}

impl IssueStatus {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "resolved" => Self::Resolved,
            "ignored" | "archived" | "muted" => Self::Ignored,
            _ => Self::Unresolved,
        }
    }

    pub fn as_api_str(&self) -> &'static str {
        match self {
            IssueStatus::Unresolved => "unresolved",
            IssueStatus::Resolved => "resolved",
            IssueStatus::Ignored => "ignored",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            IssueStatus::Unresolved => "UNRESOLVED",
            IssueStatus::Resolved => "RESOLVED",
            IssueStatus::Ignored => "IGNORED",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssuePriority {
    High,
    Medium,
    Low,
    #[default]
    None,
}

impl IssuePriority {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s.unwrap_or("").trim().to_ascii_lowercase().as_str() {
            "high" | "urgent" | "critical" => Self::High,
            "medium" | "med" => Self::Medium,
            "low" => Self::Low,
            _ => Self::None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            IssuePriority::High => "HIGH",
            IssuePriority::Medium => "MED",
            IssuePriority::Low => "LOW",
            IssuePriority::None => "—",
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            IssuePriority::High => 3,
            IssuePriority::Medium => 2,
            IssuePriority::Low => 1,
            IssuePriority::None => 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IssueSort {
    #[default]
    LastSeen,
    Events,
    Users,
    Priority,
    FirstSeen,
}

impl IssueSort {
    pub fn next(&self) -> Self {
        match self {
            IssueSort::LastSeen => IssueSort::Events,
            IssueSort::Events => IssueSort::Users,
            IssueSort::Users => IssueSort::Priority,
            IssueSort::Priority => IssueSort::FirstSeen,
            IssueSort::FirstSeen => IssueSort::LastSeen,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            IssueSort::LastSeen => "Last Seen",
            IssueSort::Events => "Events",
            IssueSort::Users => "Users",
            IssueSort::Priority => "Priority",
            IssueSort::FirstSeen => "First Seen",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentryProject {
    pub id: ProjectId,
    pub slug: CompactString,
    pub name: CompactString,
    pub platform: Option<CompactString>,
    pub team_slug: Option<CompactString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentryIssue {
    pub id: IssueId,
    pub short_id: CompactString,
    pub title: CompactString,
    pub culprit: CompactString,
    pub permalink: CompactString,
    pub level: IssueLevel,
    pub status: IssueStatus,
    pub substatus: Option<CompactString>,
    pub priority: IssuePriority,
    pub is_unhandled: bool,
    pub is_bookmarked: bool,
    pub has_seen: bool,
    pub count: u64,
    pub user_count: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub project_id: ProjectId,
    pub project_slug: CompactString,
    pub project_name: CompactString,
    pub platform: Option<CompactString>,
    pub logger: Option<CompactString>,
    pub error_type: Option<CompactString>,
    pub error_value: Option<CompactString>,
    pub sdk_name: Option<CompactString>,
    pub assigned_to: Option<CompactString>,
    pub num_comments: u32,
    #[serde(default)]
    pub stats_24h: Vec<(i64, u64)>,
    #[serde(default)]
    pub raw_json: Option<String>,
}

impl SentryIssue {
    pub fn events_24h(&self) -> u64 {
        self.stats_24h.iter().map(|(_, c)| *c).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventTag {
    pub key: CompactString,
    pub value: CompactString,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackFrame {
    pub filename: Option<CompactString>,
    pub abs_path: Option<CompactString>,
    pub module: Option<CompactString>,
    pub function: Option<CompactString>,
    pub line_no: Option<i64>,
    pub col_no: Option<i64>,
    pub in_app: bool,
    pub package: Option<CompactString>,
    #[serde(default)]
    pub context_lines: Vec<(i64, String)>,
    #[serde(default)]
    pub vars: Vec<(CompactString, String)>,
}

impl StackFrame {
    pub fn display_location(&self) -> String {
        let file = self
            .filename
            .as_deref()
            .or(self.abs_path.as_deref())
            .or(self.module.as_deref())
            .unwrap_or("<unknown>");
        match (self.line_no, self.col_no) {
            (Some(l), Some(c)) => format!("{file}:{l}:{c}"),
            (Some(l), None) => format!("{file}:{l}"),
            _ => file.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionValue {
    pub exc_type: CompactString,
    pub value: String,
    pub module: Option<CompactString>,
    pub mechanism_type: Option<CompactString>,
    pub handled: Option<bool>,
    #[serde(default)]
    pub frames: Vec<StackFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreadcrumbEntry {
    pub timestamp: Option<CompactString>,
    pub crumb_type: CompactString,
    pub category: CompactString,
    pub level: CompactString,
    pub message: String,
    #[serde(default)]
    pub data_fields: Vec<(CompactString, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEntry {
    pub method: Option<CompactString>,
    pub url: Option<String>,
    pub inferred_content_type: Option<CompactString>,
    pub fragment: Option<String>,
    pub data_body: Option<String>,
    #[serde(default)]
    pub query: Vec<(CompactString, String)>,
    #[serde(default)]
    pub headers: Vec<(CompactString, String)>,
    #[serde(default)]
    pub cookies: Vec<(CompactString, String)>,
    #[serde(default)]
    pub env: Vec<(CompactString, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserContext {
    pub id: Option<CompactString>,
    pub email: Option<CompactString>,
    pub username: Option<CompactString>,
    pub ip_address: Option<CompactString>,
    pub name: Option<CompactString>,
    pub geo: Option<CompactString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentryEventDetail {
    pub issue_id: IssueId,
    pub event_id: EventId,
    pub date_created: DateTime<Utc>,
    pub date_received: Option<DateTime<Utc>>,
    pub title: CompactString,
    pub message: String,
    pub culprit: CompactString,
    pub platform: Option<CompactString>,
    pub release: Option<CompactString>,
    pub environment: Option<CompactString>,
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub fingerprints: Vec<CompactString>,
    #[serde(default)]
    pub tags: Vec<EventTag>,
    #[serde(default)]
    pub exceptions: Vec<ExceptionValue>,
    pub formatted_message: Option<String>,
    #[serde(default)]
    pub breadcrumbs: Vec<BreadcrumbEntry>,
    pub request: Option<RequestEntry>,
    pub user: Option<UserContext>,
    #[serde(default)]
    pub custom_context: Vec<(CompactString, String)>,
    #[serde(default)]
    pub runtime_contexts: Vec<(CompactString, Vec<(CompactString, String)>)>,
    pub sdk_info: Option<CompactString>,
    #[serde(default)]
    pub packages: Vec<(CompactString, CompactString)>,
    pub raw_json: String,
}

impl SentryEventDetail {
    pub fn total_frames(&self) -> usize {
        self.exceptions.iter().map(|e| e.frames.len()).sum()
    }

    pub fn in_app_frames(&self) -> usize {
        self.exceptions
            .iter()
            .flat_map(|e| e.frames.iter())
            .filter(|f| f.in_app)
            .count()
    }

    pub fn format_markdown_summary(&self, issue: &SentryIssue) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# [{}] {}\n",
            issue.short_id, issue.title
        ));
        out.push_str(&format!(
            "- **Project:** `{}` | **Level:** `{}` | **Status:** `{}` | **Priority:** `{}`\n",
            issue.project_slug,
            issue.level.label(),
            issue.status.label(),
            issue.priority.label()
        ));
        out.push_str(&format!(
            "- **Events:** {} (24h: {}) | **Users:** {} | **Unhandled:** {}\n",
            issue.count,
            issue.events_24h(),
            issue.user_count,
            issue.is_unhandled
        ));
        if let Some(ref env) = self.environment {
            out.push_str(&format!("- **Environment:** `{env}`\n"));
        }
        out.push_str(&format!("- **URL:** {}\n\n", issue.permalink));

        if !self.exceptions.is_empty() {
            out.push_str("## Exceptions & Stacktrace\n");
            for exc in &self.exceptions {
                out.push_str(&format!("### `{}`: {}\n", exc.exc_type, exc.value));
                if !exc.frames.is_empty() {
                    out.push_str("```text\n");
                    for f in exc.frames.iter().rev().take(20) {
                        let tag = if f.in_app { "[app]" } else { "[sys]" };
                        let func = f.function.as_deref().unwrap_or("<anonymous>");
                        out.push_str(&format!(
                            "{tag} {} in {func}\n",
                            f.display_location()
                        ));
                    }
                    out.push_str("```\n");
                }
            }
        } else if let Some(ref msg) = self.formatted_message {
            out.push_str(&format!("## Message\n```text\n{msg}\n```\n"));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityKind {
    NewIssue,
    EventSpike { delta: u64, total: u64 },
    Regressed,
    StatusChanged { from: IssueStatus, to: IssueStatus },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub id: ActivityId,
    pub issue_id: IssueId,
    pub short_id: CompactString,
    pub project_slug: CompactString,
    pub issue_title: CompactString,
    pub level: IssueLevel,
    pub is_unhandled: bool,
    pub kind: ActivityKind,
    pub summary: CompactString,
    pub timestamp: DateTime<Utc>,
    pub permalink: CompactString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainChange {
    pub issue_id: IssueId,
    pub short_id: CompactString,
    pub project_slug: CompactString,
    pub issue_title: CompactString,
    pub summary: CompactString,
    pub url: CompactString,
    pub level: IssueLevel,
    pub is_critical: bool,
    pub delta_events: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentryMutation {
    UpdateStatus {
        issue_id: IssueId,
        short_id: CompactString,
        status: IssueStatus,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HydrationState {
    #[default]
    ColdBootstrap,
    CatchUpDelta,
    LiveSteadyState,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMeta {
    pub last_sync_at: Option<DateTime<Utc>>,
    pub org_slug: Option<CompactString>,
}
