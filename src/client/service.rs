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
}

impl SentryService {
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        let api = SentryApiClient::new(config)?;
        Ok(Self {
            api,
            has_completed_initial: false,
        })
    }

    pub fn update_config(&mut self, config: ClientConfig) {
        self.api.update_config(config);
    }

    pub async fn sync(&mut self, sender: &Sender<SlimEvent>) -> Result<(), ClientError> {
        if self.api.config().offline {
            return Err(ClientError::OfflineMode);
        }

        let projects = self.api.fetch_projects().await.unwrap_or_default();

        // Fetch unresolved issues + recently active issues across configured projects
        let query = if self.api.config().configured_projects.is_empty() {
            None
        } else {
            let proj_filters: Vec<String> = self
                .api
                .config()
                .configured_projects
                .iter()
                .map(|p| format!("project:{p}"))
                .collect();
            if proj_filters.len() == 1 {
                Some(proj_filters[0].clone())
            } else {
                None
            }
        };

        let mut issues = self.api.fetch_issues(query.as_deref()).await?;

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
        let detail = self.api.fetch_latest_event(issue_id).await?;
        sender.dispatch(SlimEvent::IssueEventLoaded(Box::new(detail)));
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
