use chrono::{DateTime, Local, Utc};
use compact_str::{CompactString, format_compact};

pub mod fx;
pub mod popup;
pub mod stateful_widgets;
pub mod widget;

pub fn format_relative_time(dt: DateTime<Utc>) -> CompactString {
    let now = Utc::now();
    let secs = now.signed_duration_since(dt).num_seconds();
    if secs < 10 {
        return CompactString::from("now");
    }
    if secs < 60 {
        return format_compact!("{}s", secs);
    }
    let mins = secs / 60;
    if mins < 60 {
        return format_compact!("{}m", mins);
    }
    let hours = mins / 60;
    if hours < 24 {
        return format_compact!("{}h", hours);
    }
    let days = hours / 24;
    if days < 14 {
        return format_compact!("{}d", days);
    }
    let local: DateTime<Local> = dt.with_timezone(&Local);
    CompactString::from(local.format("%d %b").to_string())
}

pub fn format_compact_count(n: u64) -> CompactString {
    if n >= 1_000_000 {
        format_compact!("{:.1}M", (n as f64) / 1_000_000.0)
    } else if n >= 10_000 {
        format_compact!("{}k", n / 1_000)
    } else if n >= 1_000 {
        format_compact!("{:.1}k", (n as f64) / 1_000.0)
    } else {
        format_compact!("{}", n)
    }
}

pub fn render_sparkline_24h(stats: &[(i64, u64)], width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if stats.is_empty() || width == 0 {
        return "·".repeat(width);
    }
    let slice = if stats.len() > width {
        &stats[stats.len() - width..]
    } else {
        stats
    };
    let max_val = slice.iter().map(|(_, c)| *c).max().unwrap_or(0);
    if max_val == 0 {
        return "·".repeat(slice.len());
    }
    let mut out = String::with_capacity(slice.len() * 3);
    for &(_, c) in slice {
        if c == 0 {
            out.push('·');
        } else {
            let idx = (((c as f64) / (max_val as f64)) * 7.0).round() as usize;
            out.push(BARS[idx.min(7)]);
        }
    }
    out
}
