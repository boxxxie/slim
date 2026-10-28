use std::{path::PathBuf, sync::mpsc::Sender};

use compact_str::CompactString;
use directories::ProjectDirs;
use tracing::{Level, Metadata};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

use crate::event::SlimEvent;

#[derive(Debug, Clone)]
pub struct LoggingConfig {
    pub file_level: Level,
    pub log_dir: Option<PathBuf>,
    pub json_format: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            file_level: Level::DEBUG,
            log_dir: Some(Self::default_log_dir()),
            json_format: false,
        }
    }
}

impl LoggingConfig {
    pub fn default_log_dir() -> PathBuf {
        if let Some(proj_dirs) = ProjectDirs::from("", "", "slim") {
            proj_dirs.cache_dir().to_path_buf()
        } else {
            PathBuf::from("slim-logs")
        }
    }

    pub fn from_env(debug_flag: bool) -> Self {
        let mut config = Self::default();
        if debug_flag {
            config.file_level = Level::TRACE;
        }
        if let Ok(level) = std::env::var("SLIM_LOG_LEVEL") {
            if let Ok(parsed) = level.parse::<Level>() {
                config.file_level = parsed;
            }
        }
        if let Ok(log_dir) = std::env::var("SLIM_LOG_DIR") {
            config.log_dir = Some(PathBuf::from(log_dir));
        }
        if std::env::var("SLIM_NO_FILE_LOGS").is_ok() {
            config.log_dir = None;
        }
        if std::env::var("SLIM_JSON_LOGS").is_ok() {
            config.json_format = true;
        }
        config
    }
}

pub struct InternalLogsLayer {
    sender: Sender<SlimEvent>,
    min_level: Level,
}

impl InternalLogsLayer {
    pub fn new(sender: Sender<SlimEvent>, min_level: Level) -> Self {
        Self { sender, min_level }
    }
}

impl<S> Layer<S> for InternalLogsLayer
where
    S: tracing::Subscriber,
{
    fn enabled(
        &self,
        metadata: &Metadata<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        metadata.level() <= &self.min_level
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = LogMessageVisitor::new();
        event.record(&mut visitor);

        if let Some(message) = visitor.into_message() {
            let _ = self.sender.send(SlimEvent::LogEntry(message));
        }
    }
}

struct LogMessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl LogMessageVisitor {
    fn new() -> Self {
        Self {
            message: None,
            fields: Vec::new(),
        }
    }

    fn is_sensitive(field_name: &str) -> bool {
        let lower = field_name.to_ascii_lowercase();
        lower.contains("token") || lower.contains("authorization") || lower.contains("api_key")
    }

    fn into_message(self) -> Option<CompactString> {
        match (self.message, self.fields.is_empty()) {
            (Some(msg), true) => Some(msg.into()),
            (Some(msg), false) => Some(format!("{msg} ({})", self.fields.join(", ")).into()),
            (None, false) => Some(self.fields.join(", ").into()),
            (None, true) => None,
        }
    }
}

impl tracing::field::Visit for LogMessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if Self::is_sensitive(field.name()) {
            self.fields.push(format!("{}=[REDACTED]", field.name()));
            return;
        }
        let formatted = format!("{value:?}").trim_matches('"').to_string();
        if field.name() == "message" {
            self.message = Some(formatted);
        } else {
            self.fields.push(format!("{}={formatted}", field.name()));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if Self::is_sensitive(field.name()) {
            self.fields.push(format!("{}=[REDACTED]", field.name()));
            return;
        }
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

pub fn init_logging(
    config: LoggingConfig,
    event_sender: Option<Sender<SlimEvent>>,
) -> Result<Option<WorkerGuard>, Box<dyn std::error::Error>> {
    let mut layers = Vec::new();
    let mut guard = None;

    if let Some(log_dir) = &config.log_dir {
        std::fs::create_dir_all(log_dir)?;
        let file_appender = tracing_appender::rolling::daily(log_dir, "slim.log");
        let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);
        guard = Some(file_guard);

        let file_filter = EnvFilter::builder()
            .with_default_directive(config.file_level.into())
            .from_env_lossy();

        let file_layer = if config.json_format {
            fmt::layer()
                .json()
                .with_writer(non_blocking)
                .with_filter(file_filter)
                .boxed()
        } else {
            fmt::layer()
                .with_writer(non_blocking)
                .with_filter(file_filter)
                .boxed()
        };

        layers.push(file_layer);
    }

    if let Some(sender) = event_sender {
        let internal_layer = InternalLogsLayer::new(sender, Level::INFO).boxed();
        layers.push(internal_layer);
    }

    let _ = tracing_subscriber::registry().with(layers).try_init();
    Ok(guard)
}
