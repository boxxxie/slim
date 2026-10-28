use std::collections::VecDeque;

use compact_str::CompactString;

use crate::{event::SlimEvent, result::SlimError};

#[derive(Debug, Default)]
pub struct NoticeService {
    info_notices: VecDeque<Notice>,
    error_notices: VecDeque<Notice>,
    most_recent: Option<Notice>,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub level: NoticeLevel,
    pub message: NoticeMessage,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum NoticeLevel {
    Info,
    Error,
}

#[derive(Debug, Clone)]
pub enum NoticeMessage {
    GeneralMessage(CompactString),
    InvalidSentryToken,
    ConfigError(CompactString),
    SyncError(CompactString),
}

impl NoticeService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: &SlimEvent) {
        match event {
            SlimEvent::AppError(err) => {
                let msg = match err {
                    SlimError::InvalidSentryToken => NoticeMessage::InvalidSentryToken,
                    SlimError::ConfigFileNotFound { path } => NoticeMessage::ConfigError(
                        format!("Configuration file not found: {}", path.display()).into(),
                    ),
                    SlimError::ConfigLoadError { path, message } => NoticeMessage::ConfigError(
                        format!("Failed to load config from {}: {message}", path.display()).into(),
                    ),
                    SlimError::ConfigSaveError { path, message } => NoticeMessage::ConfigError(
                        format!("Failed to save config to {}: {message}", path.display()).into(),
                    ),
                    SlimError::ConfigValidationError { field, message } => {
                        NoticeMessage::ConfigError(format!("Invalid {field}: {message}").into())
                    }
                    SlimError::DatabaseError(s) => {
                        NoticeMessage::GeneralMessage(format!("DuckDB: {s}").into())
                    }
                    SlimError::Client(c) => NoticeMessage::SyncError(c.to_string().into()),
                    SlimError::GeneralError(s) => NoticeMessage::GeneralMessage(s.clone()),
                };
                self.push_notice(NoticeLevel::Error, msg);
            }
            SlimEvent::SyncError(msg) | SlimEvent::MutationFailed(msg) => {
                self.push_notice(NoticeLevel::Error, NoticeMessage::SyncError(msg.clone()));
            }
            SlimEvent::ActivityNotice(msg) => {
                self.push_notice(
                    NoticeLevel::Info,
                    NoticeMessage::GeneralMessage(msg.clone()),
                );
            }
            _ => {}
        }
    }

    pub fn has_error(&self) -> bool {
        !self.error_notices.is_empty()
    }

    pub fn last_notification(&self) -> Option<&Notice> {
        self.most_recent.as_ref()
    }

    pub fn pop_notice(&mut self) -> Option<Notice> {
        let notice = self
            .error_notices
            .pop_front()
            .or_else(|| self.info_notices.pop_front());

        if notice.is_some() {
            self.most_recent = notice.clone();
        }

        notice
    }

    pub fn push_notice(&mut self, level: NoticeLevel, message: NoticeMessage) {
        let notice = Notice { level, message };
        match level {
            NoticeLevel::Info => self.info_notices.push_back(notice),
            NoticeLevel::Error => self.error_notices.push_back(notice),
        }
    }
}
