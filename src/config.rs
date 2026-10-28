use std::path::{Path, PathBuf};

use compact_str::CompactString;
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};

use crate::{
    domain::ListViewKind,
    result::{Result, SlimError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    EnvVar(CompactString),
    ConfigFile(CompactString),
    SentryCliFallback(CompactString),
}

impl SecretSource {
    pub fn as_str(&self) -> &str {
        match self {
            Self::EnvVar(s) | Self::ConfigFile(s) | Self::SentryCliFallback(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentrySection {
    #[serde(default)]
    pub auth_token: Option<CompactString>,
    #[serde(default = "default_base_url")]
    pub base_url: CompactString,
    #[serde(default = "default_org")]
    pub org: CompactString,
    #[serde(default)]
    pub projects: Vec<CompactString>,
}

fn default_base_url() -> CompactString {
    CompactString::from("https://sentry.io")
}

fn default_org() -> CompactString {
    read_sentryclirc_field("defaults", "org")
        .unwrap_or_else(|| CompactString::from("my-org"))
}

impl Default for SentrySection {
    fn default() -> Self {
        Self {
            auth_token: None,
            base_url: default_base_url(),
            org: default_org(),
            projects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewsSection {
    #[serde(default)]
    pub default_view: ListViewKind,
    #[serde(default = "default_true")]
    pub show_sparklines: bool,
}

impl Default for ViewsSection {
    fn default() -> Self {
        Self {
            default_view: ListViewKind::Unresolved,
            show_sparklines: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollingSection {
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
    #[serde(default = "default_issues_per_poll")]
    pub max_issues_per_poll: usize,
    #[serde(default = "default_max_stored_issues")]
    pub max_stored_issues: usize,
    #[serde(default = "default_max_activity_events")]
    pub max_activity_events: usize,
}

fn default_poll_seconds() -> u64 {
    30
}

fn default_issues_per_poll() -> usize {
    100
}

fn default_max_stored_issues() -> usize {
    500
}

fn default_max_activity_events() -> usize {
    300
}

impl Default for PollingSection {
    fn default() -> Self {
        Self {
            poll_seconds: default_poll_seconds(),
            max_issues_per_poll: default_issues_per_poll(),
            max_stored_issues: default_max_stored_issues(),
            max_activity_events: default_max_activity_events(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSection {
    #[serde(default = "default_true")]
    pub quack_server_enabled: bool,
    #[serde(default = "default_quack_port")]
    pub quack_port: u16,
}

fn default_true() -> bool {
    true
}

fn default_quack_port() -> u16 {
    9495
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            quack_server_enabled: true,
            quack_port: default_quack_port(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationsSection {
    #[serde(default = "default_notification_check_seconds")]
    pub notification_check_seconds: u64,
    #[serde(default = "default_true")]
    pub critical_unhandled_alert: bool,
    #[serde(default = "default_true")]
    pub notify_on_new_issues: bool,
    #[serde(default = "default_true")]
    pub notify_on_regressions: bool,
    #[serde(default = "default_spike_min_delta")]
    pub spike_min_delta: u64,
    #[serde(default = "default_true")]
    pub per_issue_dedup: bool,
    #[serde(default = "default_coalesce_threshold")]
    pub coalesce_threshold: usize,
    #[serde(default = "default_true")]
    pub desktop_notifications_enabled: bool,
}

fn default_notification_check_seconds() -> u64 {
    10
}

fn default_spike_min_delta() -> u64 {
    5
}

fn default_coalesce_threshold() -> usize {
    3
}

impl Default for NotificationsSection {
    fn default() -> Self {
        Self {
            notification_check_seconds: default_notification_check_seconds(),
            critical_unhandled_alert: true,
            notify_on_new_issues: true,
            notify_on_regressions: true,
            spike_min_delta: default_spike_min_delta(),
            per_issue_dedup: true,
            coalesce_threshold: default_coalesce_threshold(),
            desktop_notifications_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectsSection {
    #[serde(default = "default_true")]
    pub animations_enabled: bool,
    #[serde(default = "default_in_app_timeout_ms")]
    pub in_app_notification_timeout_ms: u64,
}

fn default_in_app_timeout_ms() -> u64 {
    5000
}

impl Default for EffectsSection {
    fn default() -> Self {
        Self {
            animations_enabled: true,
            in_app_notification_timeout_ms: default_in_app_timeout_ms(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlimConfig {
    #[serde(default)]
    pub sentry: SentrySection,
    #[serde(default)]
    pub views: ViewsSection,
    #[serde(default)]
    pub polling: PollingSection,
    #[serde(default)]
    pub database: DatabaseSection,
    #[serde(default)]
    pub notifications: NotificationsSection,
    #[serde(default)]
    pub effects: EffectsSection,
}

pub fn default_config_path() -> PathBuf {
    if let Some(dirs) = BaseDirs::new() {
        dirs.config_dir().join("slim.toml")
    } else {
        PathBuf::from("slim.toml")
    }
}

pub fn default_data_dir() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("", "", "slim") {
        proj_dirs.data_local_dir().to_path_buf()
    } else if let Some(dirs) = BaseDirs::new() {
        dirs.data_local_dir().join("slim")
    } else {
        PathBuf::from(".slim-data")
    }
}

pub fn default_db_path() -> PathBuf {
    default_data_dir().join("slim.duckdb")
}

pub fn default_quack_token_path() -> PathBuf {
    default_data_dir().join("quack.token")
}

pub fn load_config(path: &Path) -> Result<SlimConfig> {
    if !path.exists() {
        return Ok(SlimConfig::default());
    }
    confy::load_path(path).map_err(|e| SlimError::ConfigLoadError {
        path: path.to_path_buf(),
        message: e.to_string().into(),
    })
}

/// Resolves the Sentry Auth Token in priority order:
/// 1. `SENTRY_AUTH_TOKEN` environment variable
/// 2. `sentry.auth_token` in `~/.config/slim.toml`
/// 3. Read-only fallback to `~/.sentryclirc` (`[auth] token=...`)
pub fn resolve_auth_token(config: &SlimConfig) -> Option<SecretSource> {
    if let Ok(env_key) = std::env::var("SENTRY_AUTH_TOKEN") {
        let trimmed = env_key.trim();
        if !trimmed.is_empty() {
            return Some(SecretSource::EnvVar(trimmed.into()));
        }
    }

    if let Some(ref cfg_key) = config.sentry.auth_token {
        let trimmed = cfg_key.trim();
        if !trimmed.is_empty() {
            return Some(SecretSource::ConfigFile(trimmed.into()));
        }
    }

    if let Some(cli_key) = read_sentryclirc_field("auth", "token") {
        return Some(SecretSource::SentryCliFallback(cli_key));
    }

    None
}

pub fn read_sentryclirc_field(section: &str, key: &str) -> Option<CompactString> {
    let base = BaseDirs::new()?;
    let path = base.home_dir().join(".sentryclirc");
    let raw = std::fs::read_to_string(path).ok()?;
    parse_ini_field(&raw, section, key)
}

pub fn parse_ini_field(raw: &str, target_section: &str, target_key: &str) -> Option<CompactString> {
    let mut current_section = "";
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = &trimmed[1..trimmed.len() - 1];
            continue;
        }
        if current_section.eq_ignore_ascii_case(target_section) {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case(target_key) {
                    let val = v.trim().trim_matches('"').trim_matches('\'');
                    if !val.is_empty() {
                        return Some(CompactString::from(val));
                    }
                }
            }
        }
    }
    None
}

pub fn save_config(path: &Path, config: &SlimConfig, persist_token: bool) -> Result<()> {
    let mut sanitized = config.clone();
    if !persist_token {
        if let Ok(env_key) = std::env::var("SENTRY_AUTH_TOKEN") {
            if sanitized.sentry.auth_token.as_deref() == Some(env_key.trim()) {
                sanitized.sentry.auth_token = None;
            }
        }
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    confy::store_path(path, &sanitized)
        .map_err(|e| SlimError::config_save_error(path.to_path_buf(), e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ini_field() {
        let sample = r#"
[auth]
token=test_token_123

[defaults]
org=example-org
"#;
        assert_eq!(
            parse_ini_field(sample, "auth", "token").as_deref(),
            Some("test_token_123")
        );
        assert_eq!(
            parse_ini_field(sample, "defaults", "org").as_deref(),
            Some("example-org")
        );
    }
}
