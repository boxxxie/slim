use compact_str::CompactString;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum ClientError {
    #[error("Offline mode enabled")]
    OfflineMode,

    #[error("Authentication failed (401/403) — check SENTRY_AUTH_TOKEN or ~/.sentryclirc")]
    Authentication,

    #[error("Rate limited by Sentry API (429)")]
    RateLimited,

    #[error("HTTP request error: {0}")]
    Http(CompactString),

    #[error("Failed to parse Sentry response: {0}")]
    Parse(CompactString),

    #[error("Sentry API error ({status}): {message}")]
    Api {
        status: u16,
        message: CompactString,
    },
}
