# `slim` — Single-Page Sentry TUI Monitor & Multi-Page Issue Inspector

`slim` is a fast, keyboard-driven Terminal User Interface (TUI) for monitoring [Sentry](https://sentry.io) notifications and inspecting large error events across multiple projects in the terminal, inspired by [`glim`](https://github.com/boxxxie/glim) and [`llim`](https://github.com/boxxxie/llim).

Unlike 2-tile split-screen monitors, `slim` uses a dedicated **full-screen single-page layout** for browsing incoming issues and switches to a **5-page full-screen Issue Detail Inspector** when opening an issue so large stacktraces, breadcrumbs, HTTP request payloads, and runtime contexts have the entire terminal viewport.

---

## Features

- **Full-Screen Single-Page List Views (`1`–`4`)**:
  - **`1: Unresolved`**: Active unresolved issues with 24-hour Unicode sparklines (`▁▂▄█`), total & 24h event counts, affected users, priority, and unhandled `[!]` badges.
  - **`2: Unhandled/High`**: Filters to unhandled exceptions, `FATAL` errors, `HIGH` priority issues, and `regressed` / `escalating` substatuses.
  - **`3: Incoming Feed`**: Live chronological feed of new issues, regressions, event spikes (`+N new events`), and status transitions.
  - **`4: All`**: All cached issues across statuses (`unresolved`, `resolved`, `ignored`).
- **Full-Screen 5-Page Issue Detail Inspector (`Enter` to open, `Esc` to return)**:
  - **`1: Overview & Tags`**: Issue metadata, latest event properties, error/message summary, 24-hour activity sparkline + hourly breakdown, and all event tags.
  - **`2: Exception & Stacktrace`**: Causal exception chain and stacktrace frames with interactive toggles for In-App vs All frames (`a`) and local variables + source context lines (`v`).
  - **`3: Breadcrumbs`**: Chronological event breadcrumbs + local monitor activity timeline.
  - **`4: Request & Contexts`**: Custom extra context (`event.context`), HTTP request method/URL/headers/env/body, user context, runtime/OS/trace contexts, and loaded packages.
  - **`5: Raw Event JSON`**: Line-numbered pretty-printed JSON of the full event payload.
- **Quick Triage & Clipboard**:
  - Resolve (`R`), Ignore (`I`), or Unresolve (`U`) issues directly from the list or detail inspector.
  - Copy Short ID (`y`), copy a formatted Markdown issue + stacktrace summary (`Y`), or open in browser (`o` / `w`).
- **Stochastic & Hawkes Notification Relevance Filter (`notify-send` + In-App Banners)**:
  - **Hawkes Self-Inhibition Cooldown**: Each emitted notification records a timestamped emission in DuckDB (`notification_emissions`) that exponentially dampens both the specific issue (`issue_cooldown_half_life_minutes = 45.0`) and the noisy project/app (`project_cooldown_half_life_minutes = 20.0`).
  - **Baseline-Relative Novelty & Chronic-Repeater Decay**: Scores `+delta` events relative to historical/24h baseline volume so chronic background repeaters (e.g. 40,000-event workers firing every 30s) decay toward zero probability while genuinely new errors pass immediately.
  - **User Interaction Boost (`user_interactions`)**: Issues and projects you inspect (`Enter`), open in browser (`o`), copy (`y`/`Y`), or unresolve (`U`) receive a Hawkes relevance boost, while ignored issues (`I`) receive strong negative suppression.
  - **Stochastic Gumbel-Top-k & Logistic Gate (`m` to cycle modes)**: Converts net relevance (`raw_signal + user_boost - issue_fatigue - project_fatigue`) into a logistic emission probability, caps notifications per flush window (`max_notifications_per_flush = 2`), and automatically downgrades `critical` desktop urgency on repetitive unhandled issues.
  - **Full Raw Stream Preserved**: Suppressed notifications never drop data—all activity events remain visible in the `3: Incoming Feed` tab.
- **DuckDB 2.0 + Quack Server (`~/.local/share/slim/slim.duckdb`, port `9495`)**:
  - Instant offline startup from local DuckDB cache and concurrent SQL inspection via `slim --print-quack-attach`.

---

## Installation

```bash
cargo install --path .
```

Ensure `duckdb` (with `quack` extension support) is installed and available on your `PATH` (or `~/.local/bin/duckdb`).

---

## Configuration (`~/.config/slim.toml`)

`slim` resolves your Sentry Auth Token and Organization in priority order:
1. `SENTRY_AUTH_TOKEN` environment variable
2. `sentry.auth_token` and `sentry.org` in `~/.config/slim.toml`
3. Automatic fallback to `~/.sentryclirc` (`[auth] token` and `[defaults] org`)

Example `~/.config/slim.toml`:

```toml
[sentry]
base_url = "https://sentry.io"
org = "my-org"
projects = [] # Empty watches all projects in the organization

[views]
default_view = "unresolved"
show_sparklines = true

[polling]
poll_seconds = 30
max_issues_per_poll = 100

[database]
quack_server_enabled = true
quack_port = 9495

[notifications]
notification_check_seconds = 10
critical_unhandled_alert = true
notify_on_new_issues = true
notify_on_regressions = true
spike_min_delta = 5
desktop_notifications_enabled = true
stochastic_filter_enabled = true
filter_mode = "hybrid" # "hybrid", "stochastic", "deterministic", or "legacy"
stochastic_temperature = 0.75
relevance_threshold = 18.0
max_notifications_per_flush = 2
issue_cooldown_half_life_minutes = 45.0
project_cooldown_half_life_minutes = 20.0
issue_fatigue_weight = 32.0
project_fatigue_weight = 10.0
interaction_half_life_hours = 24.0
interaction_weight = 1.0
```

---

## CLI Usage

```bash
# Launch interactive TUI
slim

# Verify configuration, token resolution, and DuckDB path
slim --verify-config

# Run a single non-interactive sync into DuckDB and exit
slim --sync-once

# Non-interactively print a formatted Markdown summary + stacktrace for an issue
slim --inspect ACME-API-42

# Print DuckDB Quack ATTACH snippet for concurrent SQL queries
slim --print-quack-attach

# Launch in offline mode using cached DuckDB rows
slim --offline
```

## License

MIT
