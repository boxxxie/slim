use compact_str::CompactString;

use crate::config::{SlimConfig, resolve_auth_token};

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub base_url: CompactString,
    pub org: CompactString,
    pub auth_token: Option<CompactString>,
    pub configured_projects: Vec<CompactString>,
    pub max_issues_per_poll: usize,
    pub offline: bool,
}

impl ClientConfig {
    pub fn from_slim_config(cfg: &SlimConfig, offline: bool) -> Self {
        let auth_token = resolve_auth_token(cfg).map(|s| CompactString::from(s.as_str()));
        Self {
            base_url: cfg.sentry.base_url.clone(),
            org: cfg.sentry.org.clone(),
            auth_token,
            configured_projects: cfg.sentry.projects.clone(),
            max_issues_per_poll: cfg.polling.max_issues_per_poll.clamp(10, 100),
            offline,
        }
    }
}
