use std::{
    io::{self, Write},
    process::{Command, Stdio},
};

use crossterm::terminal::SetTitle;
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

/// Computes the terminal window title showing unresolved issues, high/unhandled issues,
/// and unseen incoming feed events.
pub fn compute_window_title(
    unresolved_count: usize,
    high_unhandled_count: usize,
    unseen_events: usize,
    context_label: Option<&str>,
) -> String {
    let mut badges: Vec<String> = Vec::new();
    if unresolved_count > 0 {
        badges.push(format!("🔵{unresolved_count}"));
    }
    if high_unhandled_count > 0 {
        badges.push(format!("🔴{high_unhandled_count}"));
    }
    if unseen_events > 0 {
        badges.push(format!("🔔{unseen_events}"));
    }

    let suffix = context_label
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Sentry");

    if badges.is_empty() {
        format!("🐛 slim • {suffix}")
    } else {
        format!("🐛 slim ({}) • {suffix}", badges.join(" • "))
    }
}

/// Updates the terminal emulator (and tmux pane/passthrough if inside tmux) window title.
pub fn set_terminal_window_title(title: &str) {
    let clean: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();

    let mut out = io::stdout();
    let _ = crossterm::execute!(out, SetTitle(&clean));

    if std::env::var_os("TMUX").is_some() {
        let _ = write!(
            out,
            "\x1b]2;{clean}\x07\x1bPtmux;\x1b\x1b]0;{clean}\x07\x1b\\"
        );
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_window_title_for_unresolved_high_and_unseen() {
        assert_eq!(
            compute_window_title(0, 0, 0, Some("iris-ex")),
            "🐛 slim • iris-ex"
        );
        assert_eq!(
            compute_window_title(5, 0, 0, Some("iris-ex")),
            "🐛 slim (🔵5) • iris-ex"
        );
        assert_eq!(
            compute_window_title(5, 2, 3, Some("IRIS-EX-12")),
            "🐛 slim (🔵5 • 🔴2 • 🔔3) • IRIS-EX-12"
        );
        assert_eq!(
            compute_window_title(0, 0, 1, None),
            "🐛 slim (🔔1) • Sentry"
        );
    }
}
