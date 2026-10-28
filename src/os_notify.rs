use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyUrgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl NotifyUrgency {
    pub fn as_str(&self) -> &'static str {
        match self {
            NotifyUrgency::Low => "low",
            NotifyUrgency::Normal => "normal",
            NotifyUrgency::Critical => "critical",
        }
    }
}

fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn send_os_notification(
    summary: &str,
    body: &str,
    urgency: NotifyUrgency,
    timeout_ms: u64,
    url: Option<&str>,
) {
    debug!(
        summary = summary,
        body = body,
        urgency = urgency.as_str(),
        timeout_ms = timeout_ms,
        url = ?url,
        "Sending OS notification"
    );

    let summary_owned = summary.to_string();
    let body_escaped = escape_markup(body);
    let url_owned = url.filter(|u| !u.is_empty()).map(|u| u.to_string());
    let formatted_body = if let Some(ref link) = url_owned {
        format!("{body_escaped}\n<a href=\"{link}\">{link}</a>")
    } else {
        body_escaped
    };
    let urgency_str = urgency.as_str();
    let timeout_str = timeout_ms.to_string();

    std::thread::spawn(move || {
        let mut cmd = Command::new("notify-send");
        cmd.arg("-a")
            .arg("slim")
            .arg("-u")
            .arg(urgency_str)
            .arg("-t")
            .arg(&timeout_str);

        if url_owned.is_some() {
            cmd.arg("-A")
                .arg("default=Open")
                .arg("-A")
                .arg("open=Open in Browser");
        }

        cmd.arg(&summary_owned)
            .arg(&formatted_body)
            .stdin(Stdio::null())
            .stderr(Stdio::null());

        if let Some(target_url) = url_owned {
            match cmd.output() {
                Ok(out) => {
                    let chosen = String::from_utf8_lossy(&out.stdout);
                    let action = chosen.trim();
                    if action == "default" || action == "open" {
                        let _ = open::that(&target_url);
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to run notify-send for OS notification");
                }
            }
        } else {
            let status = cmd.stdout(Stdio::null()).status();
            if let Err(e) = status {
                warn!(error = %e, "Failed to run notify-send for OS notification");
            }
        }
    });
}
