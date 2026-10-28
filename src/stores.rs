use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Sender},
    thread,
};

use chrono::Utc;
use compact_str::CompactString;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    config::{DatabaseSection, NotificationsSection, default_quack_token_path},
    domain::{
        ActivityEvent, ActivityKind, DomainChange, HydrationState, IssueLevel, IssueStatus,
        SentryEventDetail, SentryIssue, SentryProject, SyncMeta,
    },
    event::{PollBatchPayload, SlimEvent},
    id::{ActivityId, IssueId, ProjectId},
};

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id VARCHAR PRIMARY KEY,
    slug VARCHAR,
    name VARCHAR,
    platform VARCHAR,
    team_slug VARCHAR,
    payload VARCHAR
);
CREATE TABLE IF NOT EXISTS issues (
    id VARCHAR PRIMARY KEY,
    short_id VARCHAR,
    project_slug VARCHAR,
    level VARCHAR,
    status VARCHAR,
    substatus VARCHAR,
    priority VARCHAR,
    is_unhandled BOOLEAN,
    count BIGINT,
    user_count BIGINT,
    title VARCHAR,
    culprit VARCHAR,
    first_seen VARCHAR,
    last_seen VARCHAR,
    permalink VARCHAR,
    payload VARCHAR
);
CREATE TABLE IF NOT EXISTS issue_events (
    issue_id VARCHAR PRIMARY KEY,
    event_id VARCHAR,
    date_created VARCHAR,
    platform VARCHAR,
    release VARCHAR,
    environment VARCHAR,
    payload VARCHAR
);
CREATE TABLE IF NOT EXISTS activity_events (
    id VARCHAR PRIMARY KEY,
    issue_id VARCHAR,
    short_id VARCHAR,
    project_slug VARCHAR,
    kind VARCHAR,
    summary VARCHAR,
    timestamp VARCHAR,
    payload VARCHAR
);
CREATE TABLE IF NOT EXISTS sync_meta (
    id INTEGER PRIMARY KEY,
    payload VARCHAR
);
"#;

pub fn resolve_duckdb_binary() -> PathBuf {
    if let Some(base) = directories::BaseDirs::new() {
        let home_duckdb = base.home_dir().join(".local").join("bin").join("duckdb");
        if home_duckdb.exists() {
            return home_duckdb;
        }
    }
    PathBuf::from("duckdb")
}

fn sql_escape(s: &str) -> String {
    s.replace('\'', "''").replace(['\r', '\n'], " ")
}

fn sql_opt_str(opt: Option<&str>) -> String {
    match opt {
        Some(s) => format!("'{}'", sql_escape(s)),
        None => "NULL".to_string(),
    }
}

#[derive(Debug)]
enum SqlWorkerMsg {
    Execute(String),
    Shutdown(mpsc::SyncSender<()>),
}

#[derive(Debug)]
pub struct DuckDbHandle {
    sql_tx: Sender<SqlWorkerMsg>,
    pub quack_listen_uri: Option<CompactString>,
    pub quack_token: Option<CompactString>,
}

impl DuckDbHandle {
    pub fn execute_async(&self, sql_batch: String) {
        if !sql_batch.trim().is_empty() {
            let _ = self.sql_tx.send(SqlWorkerMsg::Execute(sql_batch));
        }
    }

    pub fn shutdown(&self) {
        if let Some(ref uri) = self.quack_listen_uri {
            let _ = self.sql_tx.send(SqlWorkerMsg::Execute(format!(
                "CALL quack_stop('{}');",
                sql_escape(uri)
            )));
        }
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        if self.sql_tx.send(SqlWorkerMsg::Shutdown(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(5));
        }
    }
}

pub struct SentryStore {
    pub projects: HashMap<ProjectId, SentryProject>,
    pub issues: HashMap<IssueId, SentryIssue>,
    pub events_by_issue: HashMap<IssueId, SentryEventDetail>,
    pub activity_events: VecDeque<ActivityEvent>,
    pub sync_meta: SyncMeta,
    pub hydration_state: HydrationState,
    pub generation: u64,
    db_path: PathBuf,
    duckdb_handle: Option<DuckDbHandle>,
}

impl SentryStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            projects: HashMap::new(),
            issues: HashMap::new(),
            events_by_issue: HashMap::new(),
            activity_events: VecDeque::new(),
            sync_meta: SyncMeta::default(),
            hydration_state: HydrationState::ColdBootstrap,
            generation: 1,
            db_path,
            duckdb_handle: None,
        }
    }

    pub fn quack_uri(&self) -> Option<&str> {
        self.duckdb_handle
            .as_ref()
            .and_then(|h| h.quack_listen_uri.as_deref())
    }

    pub fn init_duckdb_and_load(
        &mut self,
        db_config: &DatabaseSection,
        event_sender: Option<&Sender<SlimEvent>>,
    ) {
        if let Some(parent) = self.db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let duckdb_bin = resolve_duckdb_binary();

        if self.db_path.exists() {
            self.load_existing_rows(&duckdb_bin);
        } else {
            let _ = Command::new(&duckdb_bin)
                .arg("-dark-mode")
                .arg("-batch")
                .arg(&self.db_path)
                .arg("-c")
                .arg(SCHEMA_SQL)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        self.start_persistent_worker(&duckdb_bin, db_config, event_sender);
    }

    fn load_existing_rows(&mut self, duckdb_bin: &Path) {
        let _ = Command::new(duckdb_bin)
            .arg("-dark-mode")
            .arg("-batch")
            .arg(&self.db_path)
            .arg("-c")
            .arg(SCHEMA_SQL)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        for proj in query_payloads::<SentryProject>(
            duckdb_bin,
            &self.db_path,
            "SELECT payload FROM projects;",
        ) {
            self.projects.insert(proj.id.clone(), proj);
        }
        for issue in query_payloads::<SentryIssue>(
            duckdb_bin,
            &self.db_path,
            "SELECT payload FROM issues;",
        ) {
            self.issues.insert(issue.id.clone(), issue);
        }
        for ev_detail in query_payloads::<SentryEventDetail>(
            duckdb_bin,
            &self.db_path,
            "SELECT payload FROM issue_events;",
        ) {
            self.events_by_issue
                .insert(ev_detail.issue_id.clone(), ev_detail);
        }
        for act in query_payloads::<ActivityEvent>(
            duckdb_bin,
            &self.db_path,
            "SELECT payload FROM activity_events ORDER BY timestamp DESC;",
        ) {
            self.activity_events.push_back(act);
        }
        if let Some(meta) = query_payloads::<SyncMeta>(
            duckdb_bin,
            &self.db_path,
            "SELECT payload FROM sync_meta WHERE id = 1;",
        )
        .into_iter()
        .next()
        {
            self.sync_meta = meta;
        }

        if !self.issues.is_empty() {
            self.hydration_state = HydrationState::CatchUpDelta;
        }
        self.generation = self.generation.wrapping_add(1);

        info!(
            projects = self.projects.len(),
            issues = self.issues.len(),
            cached_events = self.events_by_issue.len(),
            activity_events = self.activity_events.len(),
            "Loaded cached Sentry state from DuckDB 2.0"
        );
    }

    fn start_persistent_worker(
        &mut self,
        duckdb_bin: &Path,
        db_config: &DatabaseSection,
        event_sender: Option<&Sender<SlimEvent>>,
    ) {
        let mut child: Child = match Command::new(duckdb_bin)
            .arg("-dark-mode")
            .arg("-batch")
            .arg(&self.db_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "Failed to spawn persistent DuckDB 2.0 worker");
                return;
            }
        };

        let mut stdin: ChildStdin = match child.stdin.take() {
            Some(s) => s,
            None => return,
        };
        let stdout = child.stdout.take();

        let _ = writeln!(stdin, "{SCHEMA_SQL}");

        let mut quack_listen_uri = None;
        let mut quack_token = None;

        if db_config.quack_server_enabled {
            let uri = format!("quack:127.0.0.1:{}", db_config.quack_port);
            let token = load_or_create_quack_token();
            let init_quack_sql = format!(
                "INSTALL quack;\nLOAD quack;\nCALL quack_serve('{}', token => '{}');\nSELECT 'SLIM_QUACK_DONE';\n",
                sql_escape(&uri),
                sql_escape(&token)
            );
            if writeln!(stdin, "{init_quack_sql}").is_ok() && stdin.flush().is_ok() {
                if let Some(out) = stdout {
                    let mut reader = BufReader::new(out);
                    let mut line = String::new();
                    let mut started_ok = false;
                    while let Ok(n) = reader.read_line(&mut line) {
                        if n == 0 {
                            break;
                        }
                        if line.contains(&uri) {
                            started_ok = true;
                        }
                        if line.contains("SLIM_QUACK_DONE") {
                            break;
                        }
                        line.clear();
                    }
                    thread::spawn(move || {
                        let mut sink = String::new();
                        while let Ok(n) = reader.read_line(&mut sink) {
                            if n == 0 {
                                break;
                            }
                            sink.clear();
                        }
                    });

                    if started_ok {
                        info!(uri = %uri, "Started DuckDB 2.0 Quack protocol server");
                        quack_listen_uri = Some(uri.into());
                        quack_token = Some(token);
                    } else if let Some(tx) = event_sender {
                        let _ = tx.send(SlimEvent::ActivityNotice(
                            format!(
                                "Quack port {} in use; running DuckDB in local-only mode",
                                db_config.quack_port
                            )
                            .into(),
                        ));
                    }
                }
            }
        } else if let Some(out) = stdout {
            thread::spawn(move || {
                let mut reader = BufReader::new(out);
                let mut sink = String::new();
                while let Ok(n) = reader.read_line(&mut sink) {
                    if n == 0 {
                        break;
                    }
                    sink.clear();
                }
            });
        }

        let (sql_tx, sql_rx) = mpsc::channel::<SqlWorkerMsg>();
        thread::spawn(move || {
            let mut ack_on_exit = None;
            while let Ok(msg) = sql_rx.recv() {
                match msg {
                    SqlWorkerMsg::Execute(sql) => {
                        if writeln!(stdin, "{sql}").is_err() {
                            break;
                        }
                        let _ = stdin.flush();
                    }
                    SqlWorkerMsg::Shutdown(ack) => {
                        let _ = writeln!(stdin, "CHECKPOINT;");
                        let _ = stdin.flush();
                        ack_on_exit = Some(ack);
                        break;
                    }
                }
            }
            drop(stdin);
            let _ = child.wait();
            if let Some(ack) = ack_on_exit {
                let _ = ack.send(());
            }
        });

        self.duckdb_handle = Some(DuckDbHandle {
            sql_tx,
            quack_listen_uri,
            quack_token,
        });
    }

    pub fn shutdown(&mut self) {
        if let Some(handle) = self.duckdb_handle.take() {
            handle.shutdown();
        }
    }

    pub fn project_slugs_sorted(&self) -> Vec<CompactString> {
        let mut slugs: Vec<CompactString> = self
            .projects
            .values()
            .map(|p| p.slug.clone())
            .collect();
        for iss in self.issues.values() {
            if !slugs.iter().any(|s| s == &iss.project_slug) {
                slugs.push(iss.project_slug.clone());
            }
        }
        slugs.sort();
        slugs.dedup();
        slugs
    }

    pub fn apply_poll_batch(
        &mut self,
        payload: PollBatchPayload,
        notif_cfg: &NotificationsSection,
        max_stored_issues: usize,
        max_activity_events: usize,
    ) -> Vec<DomainChange> {
        let had_prior_issues = !self.issues.is_empty();
        let mut changes = Vec::new();
        let mut new_activities = Vec::new();
        let mut sql = String::from("BEGIN TRANSACTION;\n");

        for proj in payload.projects {
            sql.push_str(&upsert_project_sql(&proj));
            self.projects.insert(proj.id.clone(), proj);
        }

        for issue in payload.issues {
            let existing = self.issues.get(&issue.id);

            if had_prior_issues && !payload.is_initial {
                match existing {
                    None => {
                        let summary = CompactString::from(format!(
                            "New {} issue ({} events)",
                            issue.level.label(),
                            issue.count
                        ));
                        let act = ActivityEvent {
                            id: ActivityId::from(format!(
                                "new-{}-{}",
                                issue.id,
                                payload.synced_at.timestamp_millis()
                            )),
                            issue_id: issue.id.clone(),
                            short_id: issue.short_id.clone(),
                            project_slug: issue.project_slug.clone(),
                            issue_title: issue.title.clone(),
                            level: issue.level,
                            is_unhandled: issue.is_unhandled,
                            kind: ActivityKind::NewIssue,
                            summary: summary.clone(),
                            timestamp: issue.last_seen,
                            permalink: issue.permalink.clone(),
                        };
                        new_activities.push(act);

                        if notif_cfg.notify_on_new_issues {
                            changes.push(DomainChange {
                                issue_id: issue.id.clone(),
                                short_id: issue.short_id.clone(),
                                project_slug: issue.project_slug.clone(),
                                issue_title: issue.title.clone(),
                                summary,
                                url: issue.permalink.clone(),
                                level: issue.level,
                                is_critical: issue.is_unhandled
                                    || issue.level == IssueLevel::Fatal,
                                delta_events: issue.count,
                            });
                        }
                    }
                    Some(old) => {
                        let regressed = (old.status != IssueStatus::Unresolved
                            && issue.status == IssueStatus::Unresolved)
                            || (issue.substatus.as_deref() == Some("regressed")
                                && old.substatus.as_deref() != Some("regressed"));

                        if regressed {
                            let summary = CompactString::from(format!(
                                "Issue regressed ({} total events)",
                                issue.count
                            ));
                            new_activities.push(ActivityEvent {
                                id: ActivityId::from(format!(
                                    "reg-{}-{}",
                                    issue.id,
                                    payload.synced_at.timestamp_millis()
                                )),
                                issue_id: issue.id.clone(),
                                short_id: issue.short_id.clone(),
                                project_slug: issue.project_slug.clone(),
                                issue_title: issue.title.clone(),
                                level: issue.level,
                                is_unhandled: issue.is_unhandled,
                                kind: ActivityKind::Regressed,
                                summary: summary.clone(),
                                timestamp: issue.last_seen,
                                permalink: issue.permalink.clone(),
                            });
                            if notif_cfg.notify_on_regressions {
                                changes.push(DomainChange {
                                    issue_id: issue.id.clone(),
                                    short_id: issue.short_id.clone(),
                                    project_slug: issue.project_slug.clone(),
                                    issue_title: issue.title.clone(),
                                    summary,
                                    url: issue.permalink.clone(),
                                    level: issue.level,
                                    is_critical: true,
                                    delta_events: issue.count.saturating_sub(old.count),
                                });
                            }
                        } else if issue.count > old.count {
                            let delta = issue.count - old.count;
                            let summary = CompactString::from(format!(
                                "+{delta} new events (total: {})",
                                issue.count
                            ));
                            new_activities.push(ActivityEvent {
                                id: ActivityId::from(format!(
                                    "spike-{}-{}",
                                    issue.id,
                                    payload.synced_at.timestamp_millis()
                                )),
                                issue_id: issue.id.clone(),
                                short_id: issue.short_id.clone(),
                                project_slug: issue.project_slug.clone(),
                                issue_title: issue.title.clone(),
                                level: issue.level,
                                is_unhandled: issue.is_unhandled,
                                kind: ActivityKind::EventSpike {
                                    delta,
                                    total: issue.count,
                                },
                                summary: summary.clone(),
                                timestamp: issue.last_seen,
                                permalink: issue.permalink.clone(),
                            });

                            if delta >= notif_cfg.spike_min_delta.max(1)
                                || issue.is_unhandled
                                || issue.level == IssueLevel::Fatal
                            {
                                changes.push(DomainChange {
                                    issue_id: issue.id.clone(),
                                    short_id: issue.short_id.clone(),
                                    project_slug: issue.project_slug.clone(),
                                    issue_title: issue.title.clone(),
                                    summary,
                                    url: issue.permalink.clone(),
                                    level: issue.level,
                                    is_critical: issue.is_unhandled
                                        || issue.level == IssueLevel::Fatal,
                                    delta_events: delta,
                                });
                            }
                        } else if old.status != issue.status {
                            let summary = CompactString::from(format!(
                                "Status changed: {} → {}",
                                old.status.label(),
                                issue.status.label()
                            ));
                            new_activities.push(ActivityEvent {
                                id: ActivityId::from(format!(
                                    "status-{}-{}",
                                    issue.id,
                                    payload.synced_at.timestamp_millis()
                                )),
                                issue_id: issue.id.clone(),
                                short_id: issue.short_id.clone(),
                                project_slug: issue.project_slug.clone(),
                                issue_title: issue.title.clone(),
                                level: issue.level,
                                is_unhandled: issue.is_unhandled,
                                kind: ActivityKind::StatusChanged {
                                    from: old.status,
                                    to: issue.status,
                                },
                                summary,
                                timestamp: payload.synced_at,
                                permalink: issue.permalink.clone(),
                            });
                        }
                    }
                }
            }

            sql.push_str(&upsert_issue_sql(&issue));
            self.issues.insert(issue.id.clone(), issue);
        }

        // If activity_events is completely empty on initial bootstrap, seed it with the most recent issues
        if self.activity_events.is_empty() && new_activities.is_empty() {
            let mut recent: Vec<&SentryIssue> = self.issues.values().collect();
            recent.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
            for iss in recent.into_iter().take(25) {
                let ev_24h = iss.events_24h();
                let summary = if ev_24h > 0 {
                    CompactString::from(format!(
                        "+{ev_24h} events in 24h (total: {})",
                        iss.count
                    ))
                } else {
                    CompactString::from(format!("Active issue ({} total events)", iss.count))
                };
                new_activities.push(ActivityEvent {
                    id: ActivityId::from(format!("seed-{}", iss.id)),
                    issue_id: iss.id.clone(),
                    short_id: iss.short_id.clone(),
                    project_slug: iss.project_slug.clone(),
                    issue_title: iss.title.clone(),
                    level: iss.level,
                    is_unhandled: iss.is_unhandled,
                    kind: ActivityKind::EventSpike {
                        delta: ev_24h,
                        total: iss.count,
                    },
                    summary,
                    timestamp: iss.last_seen,
                    permalink: iss.permalink.clone(),
                });
            }
        }

        new_activities.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        for act in new_activities {
            sql.push_str(&upsert_activity_sql(&act));
            self.activity_events.push_front(act);
        }

        while self.activity_events.len() > max_activity_events.max(50) {
            self.activity_events.pop_back();
        }

        if self.issues.len() > max_stored_issues.max(100) {
            let mut all_ids: Vec<(IssueId, chrono::DateTime<Utc>)> = self
                .issues
                .iter()
                .map(|(k, v)| (k.clone(), v.last_seen))
                .collect();
            all_ids.sort_by(|a, b| b.1.cmp(&a.1));
            for (drop_id, _) in all_ids.into_iter().skip(max_stored_issues.max(100)) {
                self.issues.remove(&drop_id);
            }
        }

        self.sync_meta.last_sync_at = Some(payload.synced_at);
        sql.push_str(&upsert_sync_meta_sql(&self.sync_meta));
        sql.push_str("COMMIT;\n");

        if let Some(ref handle) = self.duckdb_handle {
            handle.execute_async(sql);
        }

        self.hydration_state = HydrationState::LiveSteadyState;
        self.generation = self.generation.wrapping_add(1);
        changes
    }

    pub fn apply_issue_event(&mut self, detail: SentryEventDetail) {
        let sql = format!("BEGIN TRANSACTION;\n{}COMMIT;\n", upsert_event_sql(&detail));
        self.events_by_issue.insert(detail.issue_id.clone(), detail);
        if let Some(ref handle) = self.duckdb_handle {
            handle.execute_async(sql);
        }
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn apply_status_mutation(&mut self, issue_id: &IssueId, new_status: IssueStatus) {
        if let Some(issue) = self.issues.get_mut(issue_id) {
            let old_status = issue.status;
            issue.status = new_status;
            let issue_clone = issue.clone();

            let act = ActivityEvent {
                id: ActivityId::from(format!(
                    "mut-{}-{}",
                    issue_id,
                    Utc::now().timestamp_millis()
                )),
                issue_id: issue_id.clone(),
                short_id: issue_clone.short_id.clone(),
                project_slug: issue_clone.project_slug.clone(),
                issue_title: issue_clone.title.clone(),
                level: issue_clone.level,
                is_unhandled: issue_clone.is_unhandled,
                kind: ActivityKind::StatusChanged {
                    from: old_status,
                    to: new_status,
                },
                summary: CompactString::from(format!(
                    "Status updated: {} → {}",
                    old_status.label(),
                    new_status.label()
                )),
                timestamp: Utc::now(),
                permalink: issue_clone.permalink.clone(),
            };

            let sql = format!(
                "BEGIN TRANSACTION;\n{}{}COMMIT;\n",
                upsert_issue_sql(&issue_clone),
                upsert_activity_sql(&act)
            );
            self.activity_events.push_front(act);
            if let Some(ref handle) = self.duckdb_handle {
                handle.execute_async(sql);
            }
            self.generation = self.generation.wrapping_add(1);
        }
    }
}

fn upsert_project_sql(proj: &SentryProject) -> String {
    let payload = serde_json::to_string(proj).unwrap_or_default();
    format!(
        "INSERT OR REPLACE INTO projects (id, slug, name, platform, team_slug, payload) VALUES ('{}', '{}', '{}', {}, {}, '{}');\n",
        sql_escape(proj.id.as_str()),
        sql_escape(proj.slug.as_str()),
        sql_escape(proj.name.as_str()),
        sql_opt_str(proj.platform.as_deref()),
        sql_opt_str(proj.team_slug.as_deref()),
        sql_escape(&payload)
    )
}

fn upsert_issue_sql(issue: &SentryIssue) -> String {
    let payload = serde_json::to_string(issue).unwrap_or_default();
    format!(
        "INSERT OR REPLACE INTO issues (id, short_id, project_slug, level, status, substatus, priority, is_unhandled, count, user_count, title, culprit, first_seen, last_seen, permalink, payload) VALUES ('{}', '{}', '{}', '{}', '{}', {}, '{}', {}, {}, {}, '{}', '{}', '{}', '{}', '{}', '{}');\n",
        sql_escape(issue.id.as_str()),
        sql_escape(issue.short_id.as_str()),
        sql_escape(issue.project_slug.as_str()),
        sql_escape(issue.level.label()),
        sql_escape(issue.status.as_api_str()),
        sql_opt_str(issue.substatus.as_deref()),
        sql_escape(issue.priority.label()),
        if issue.is_unhandled { "TRUE" } else { "FALSE" },
        issue.count,
        issue.user_count,
        sql_escape(issue.title.as_str()),
        sql_escape(issue.culprit.as_str()),
        sql_escape(&issue.first_seen.to_rfc3339()),
        sql_escape(&issue.last_seen.to_rfc3339()),
        sql_escape(issue.permalink.as_str()),
        sql_escape(&payload)
    )
}

fn upsert_event_sql(detail: &SentryEventDetail) -> String {
    let payload = serde_json::to_string(detail).unwrap_or_default();
    format!(
        "INSERT OR REPLACE INTO issue_events (issue_id, event_id, date_created, platform, release, environment, payload) VALUES ('{}', '{}', '{}', {}, {}, {}, '{}');\n",
        sql_escape(detail.issue_id.as_str()),
        sql_escape(detail.event_id.as_str()),
        sql_escape(&detail.date_created.to_rfc3339()),
        sql_opt_str(detail.platform.as_deref()),
        sql_opt_str(detail.release.as_deref()),
        sql_opt_str(detail.environment.as_deref()),
        sql_escape(&payload)
    )
}

fn upsert_activity_sql(act: &ActivityEvent) -> String {
    let payload = serde_json::to_string(act).unwrap_or_default();
    let kind_str = match &act.kind {
        ActivityKind::NewIssue => "new_issue",
        ActivityKind::EventSpike { .. } => "event_spike",
        ActivityKind::Regressed => "regressed",
        ActivityKind::StatusChanged { .. } => "status_changed",
    };
    format!(
        "INSERT OR REPLACE INTO activity_events (id, issue_id, short_id, project_slug, kind, summary, timestamp, payload) VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}');\n",
        sql_escape(act.id.as_str()),
        sql_escape(act.issue_id.as_str()),
        sql_escape(act.short_id.as_str()),
        sql_escape(act.project_slug.as_str()),
        kind_str,
        sql_escape(act.summary.as_str()),
        sql_escape(&act.timestamp.to_rfc3339()),
        sql_escape(&payload)
    )
}

fn upsert_sync_meta_sql(meta: &SyncMeta) -> String {
    let payload = serde_json::to_string(meta).unwrap_or_default();
    format!(
        "INSERT OR REPLACE INTO sync_meta (id, payload) VALUES (1, '{}');\n",
        sql_escape(&payload)
    )
}

fn query_payloads<T: DeserializeOwned>(duckdb_bin: &Path, db_path: &Path, sql: &str) -> Vec<T> {
    let Ok(out) = Command::new(duckdb_bin)
        .arg("-dark-mode")
        .arg("-json")
        .arg(db_path)
        .arg("-c")
        .arg(sql)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };

    if !out.status.success() {
        return Vec::new();
    }

    let Ok(rows) = serde_json::from_slice::<Vec<Value>>(&out.stdout) else {
        return Vec::new();
    };

    let mut parsed = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(payload_str) = row.get("payload").and_then(|v| v.as_str()) {
            if let Ok(entity) = serde_json::from_str::<T>(payload_str) {
                parsed.push(entity);
            }
        }
    }
    parsed
}

pub fn load_or_create_quack_token() -> CompactString {
    let path = default_quack_token_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return CompactString::from(trimmed);
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let token = format!(
        "slim-quack-{:x}",
        Utc::now().timestamp_nanos_opt().unwrap_or(9495)
    );
    let _ = std::fs::write(&path, &token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }
    CompactString::from(token)
}
