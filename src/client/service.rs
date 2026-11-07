use std::sync::mpsc::Sender;

use chrono::Utc;

use crate::{
    client::{api::SentryApiClient, config::ClientConfig, error::ClientError},
    dispatcher::Dispatcher,
    domain::SentryMutation,
    event::{PollBatchPayload, SlimEvent},
    id::IssueId,
};

pub struct SentryService {
    api: SentryApiClient,
    has_completed_initial: bool,
    has_fetched_projects: bool,
}

impl SentryService {
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        let api = SentryApiClient::new(config)?;
        Ok(Self {
            api,
            has_completed_initial: false,
            has_fetched_projects: false,
        })
    }

    pub fn needs_resync_on_config_change(&self, next: &ClientConfig) -> bool {
        self.api.config().org != next.org
            || self.api.config().base_url != next.base_url
            || self.api.config().auth_token != next.auth_token
            || self.api.config().configured_projects != next.configured_projects
            || self.api.config().max_issues_per_poll != next.max_issues_per_poll
    }

    pub fn update_config(&mut self, config: ClientConfig) {
        let org_changed =
            self.api.config().org != config.org || self.api.config().base_url != config.base_url;
        self.api.update_config(config);
        if org_changed {
            self.has_fetched_projects = false;
        }
    }

    pub fn invalidate_for_manual_refresh(&mut self) {
        self.has_fetched_projects = false;
        self.api.clear_etag_cache();
    }

    pub async fn sync(&mut self, sender: &Sender<SlimEvent>) -> Result<(), ClientError> {
        if self.api.config().offline {
            return Err(ClientError::OfflineMode);
        }

        let projects = if !self.has_fetched_projects {
            match self.api.fetch_projects_conditional().await {
                Ok(Some(projs)) => {
                    self.has_fetched_projects = true;
                    projs
                }
                Ok(None) => {
                    self.has_fetched_projects = true;
                    Vec::new()
                }
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        // Fetch unresolved issues + recently active issues across configured projects
        let query = if self.api.config().configured_projects.is_empty() {
            None
        } else if self.api.config().configured_projects.len() == 1 {
            Some(format!("project:{}", self.api.config().configured_projects[0]))
        } else {
            let joined = self
                .api
                .config()
                .configured_projects
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(",");
            Some(format!("project:[{joined}]"))
        };

        let Some(mut issues) = self.api.fetch_issues_conditional(query.as_deref()).await? else {
            // 304 Not Modified: issue list has not changed since the last poll
            self.has_completed_initial = true;
            return Ok(());
        };

        if !self.api.config().configured_projects.is_empty() {
            issues.retain(|iss| {
                self.api
                    .config()
                    .configured_projects
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(iss.project_slug.as_str()))
            });
        }

        let is_initial = !self.has_completed_initial;
        self.has_completed_initial = true;

        sender.dispatch(SlimEvent::PollBatchLoaded(Box::new(PollBatchPayload {
            projects,
            issues,
            synced_at: Utc::now(),
            is_initial,
        })));

        Ok(())
    }

    pub async fn fetch_issue_event(
        &self,
        issue_id: &IssueId,
        sender: &Sender<SlimEvent>,
    ) -> Result<(), ClientError> {
        if let Some(detail) = self.api.fetch_latest_event_conditional(issue_id).await? {
            sender.dispatch(SlimEvent::IssueEventLoaded(Box::new(detail)));
        }
        Ok(())
    }

    pub async fn execute_mutation(
        &self,
        mutation: SentryMutation,
        sender: &Sender<SlimEvent>,
    ) -> Result<(), ClientError> {
        match mutation {
            SentryMutation::UpdateStatus {
                issue_id,
                short_id,
                status,
            } => {
                let updated = self.api.update_issue_status(&issue_id, status).await?;
                sender.dispatch(SlimEvent::IssueStatusUpdated {
                    issue_id,
                    short_id,
                    status: updated,
                });
            }
        }
        Ok(())
    }
}
