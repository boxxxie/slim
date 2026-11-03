use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Duration, Utc};
use compact_str::CompactString;

use crate::{
    config::NotificationsSection,
    domain::{
        DomainChange, DomainChangeKind, IssueLevel, IssuePriority, NotificationEmission,
        NotificationFilterMode, UserInteraction, UserInteractionKind,
    },
    id::IssueId,
    notification_buffer::EmittedNotification,
    os_notify::NotifyUrgency,
};

/// Context for scoring incoming Sentry notification candidates, applying Hawkes self-inhibition
/// and user interaction boosts, and executing Gumbel-Top-k / logistic stochastic gating.
#[derive(Clone, Copy, Debug)]
pub struct NotificationRelevanceContext<'a> {
    pub config: &'a NotificationsSection,
    pub recent_emissions: &'a [NotificationEmission],
    pub user_interactions: &'a [UserInteraction],
    pub filter_mode: NotificationFilterMode,
    pub now: DateTime<Utc>,
    pub flush_nonce: u64,
}

#[derive(Clone, Debug)]
pub struct ScoredCandidateGroup {
    pub issue_id: IssueId,
    pub short_id: CompactString,
    pub project_slug: CompactString,
    pub issue_title: CompactString,
    pub url: CompactString,
    pub combined_summary: CompactString,
    pub dominant_kind: DomainChangeKind,
    pub level: IssueLevel,
    pub priority: IssuePriority,
    pub is_unhandled: bool,
    pub delta_events: u64,
    pub total_events: u64,
    pub events_24h: u64,
    pub user_count: u64,
    pub raw_signal: f64,
    pub user_boost: f64,
    pub issue_fatigue: f64,
    pub project_fatigue: f64,
    pub issue_notifs_1h: usize,
    pub net_score: f64,
    pub emission_probability: f64,
    pub gumbel_key: f64,
    pub passed_gate: bool,
    pub is_pinned_urgent: bool,
    pub urgency: NotifyUrgency,
}

#[derive(Clone, Debug, Default)]
pub struct NotificationEvaluationResult {
    pub emitted: Vec<EmittedNotification>,
    pub recorded_emissions: Vec<NotificationEmission>,
    pub suppressed_count: usize,
}

/// Fast deterministic SplitMix64 hash for seeded Gumbel and uniform noise generation.
pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn hash_issue_seed(bucket: u64, issue_id: &str, salt: u64) -> u64 {
    let mut h = splitmix64(bucket ^ salt);
    for b in issue_id.as_bytes() {
        h = splitmix64(h ^ (*b as u64));
    }
    h
}

/// Deterministic uniform variate in `(0.01, 0.99)` from `(bucket, issue_id, salt)`.
pub fn seeded_uniform(bucket: u64, issue_id: &str, salt: u64) -> f64 {
    let h = hash_issue_seed(bucket, issue_id, salt);
    0.01 + 0.98 * ((h >> 11) as f64 / ((1u64 << 53) as f64))
}

/// Bounded Gumbel(0, 1) variate in `[-1.5, 2.5]` from `(bucket, issue_id, salt)`.
pub fn seeded_gumbel(bucket: u64, issue_id: &str, salt: u64) -> f64 {
    let u = seeded_uniform(bucket, issue_id, salt);
    (-(-u.ln()).ln()).clamp(-1.5, 2.5)
}

fn deduplicate_summaries(summaries: &[&str]) -> String {
    let mut unique: Vec<&str> = Vec::new();
    for &s in summaries {
        if !unique.contains(&s) {
            unique.push(s);
        }
    }
    unique.join("; ")
}

/// Evaluates a batch of queued `DomainChange` items using Hawkes self-inhibition, relative baseline
/// novelty, user interaction excitation, and stochastic Gumbel-Top-k / logistic gating.
pub fn evaluate_notifications(
    changes: Vec<DomainChange>,
    ctx: &NotificationRelevanceContext<'_>,
) -> NotificationEvaluationResult {
    if changes.is_empty() {
        return NotificationEvaluationResult::default();
    }

    let effective_mode = if !ctx.config.stochastic_filter_enabled {
        NotificationFilterMode::Legacy
    } else {
        ctx.filter_mode
    };

    if effective_mode == NotificationFilterMode::Legacy {
        return evaluate_legacy_notifications(changes, ctx.config, ctx.now);
    }

    // 1. Group changes by issue_id
    let mut by_issue: BTreeMap<IssueId, Vec<DomainChange>> = BTreeMap::new();
    for change in changes {
        by_issue
            .entry(change.issue_id.clone())
            .or_default()
            .push(change);
    }

    // 2. Precompute Hawkes self-inhibition (fatigue) from recent notification emissions
    let issue_half_life_mins = ctx.config.issue_cooldown_half_life_minutes.max(1.0);
    let proj_half_life_mins = ctx.config.project_cooldown_half_life_minutes.max(1.0);
    let lambda_issue_fatigue = std::f64::consts::LN_2 / issue_half_life_mins;
    let lambda_proj_fatigue = std::f64::consts::LN_2 / proj_half_life_mins;
    let issue_fatigue_w = ctx.config.issue_fatigue_weight.max(0.0);
    let proj_fatigue_w = ctx.config.project_fatigue_weight.max(0.0);

    let cutoff_6h = ctx.now - Duration::hours(6);
    let cutoff_1h = ctx.now - Duration::hours(1);

    let mut issue_fatigue_map: HashMap<&str, f64> = HashMap::new();
    let mut issue_notifs_1h_map: HashMap<&str, usize> = HashMap::new();
    let mut proj_fatigue_map: HashMap<CompactString, f64> = HashMap::new();

    for em in ctx.recent_emissions {
        if em.created_at < cutoff_6h {
            continue;
        }
        let age_mins = (ctx.now - em.created_at).num_seconds().max(0) as f64 / 60.0;
        let iss_decay = issue_fatigue_w * (-lambda_issue_fatigue * age_mins).exp();
        *issue_fatigue_map.entry(em.issue_id.as_str()).or_insert(0.0) += iss_decay;

        if em.created_at >= cutoff_1h {
            *issue_notifs_1h_map.entry(em.issue_id.as_str()).or_insert(0) += 1;
        }

        let proj_decay = proj_fatigue_w * (-lambda_proj_fatigue * age_mins).exp();
        *proj_fatigue_map
            .entry(em.project_slug.to_ascii_lowercase())
            .or_insert(0.0) += proj_decay;
    }

    // 3. Precompute User Interaction Hawkes excitation
    let inter_half_life_hours = ctx.config.interaction_half_life_hours.max(1.0);
    let lambda_inter = std::f64::consts::LN_2 / inter_half_life_hours;
    let inter_mult = ctx.config.interaction_weight.max(0.0);
    let cutoff_7d = ctx.now - Duration::days(7);

    let mut user_issue_boost: HashMap<&str, f64> = HashMap::new();
    let mut user_issue_resolve_boost: HashMap<&str, f64> = HashMap::new();
    let mut user_proj_boost: HashMap<CompactString, f64> = HashMap::new();
    let mut latest_issue_status: HashMap<&str, UserInteractionKind> = HashMap::new();

    if inter_mult > 0.0 {
        for ui in ctx.user_interactions {
            if ui.created_at < cutoff_7d {
                continue;
            }
            let iss_key = ui.issue_id.as_str();
            if matches!(
                ui.kind,
                UserInteractionKind::IgnoreIssue
                    | UserInteractionKind::ResolveIssue
                    | UserInteractionKind::UnresolveIssue
            ) {
                latest_issue_status.entry(iss_key).or_insert(ui.kind);
            }

            let active_status = latest_issue_status.get(iss_key).copied();
            // If the user's latest status action on this issue was Ignore, do not let prior
            // inspections/opens cancel out the ignore penalty.
            if active_status == Some(UserInteractionKind::IgnoreIssue)
                && ui.kind != UserInteractionKind::IgnoreIssue
            {
                continue;
            }
            // If the user subsequently unresolved the issue, drop older Ignore penalties.
            if active_status == Some(UserInteractionKind::UnresolveIssue)
                && ui.kind == UserInteractionKind::IgnoreIssue
            {
                continue;
            }

            let age_hours = (ctx.now - ui.created_at).num_minutes().max(0) as f64 / 60.0;
            let decayed = ui.weight * inter_mult * (-lambda_inter * age_hours).exp();
            if ui.kind == UserInteractionKind::ResolveIssue {
                // Resolving an issue only boosts relevance if the issue subsequently regresses
                *user_issue_resolve_boost.entry(iss_key).or_insert(0.0) += decayed;
            } else {
                *user_issue_boost.entry(iss_key).or_insert(0.0) += decayed;
            }

            // Only positive interactions boost the whole project slightly; ignoring an issue
            // penalizes that specific issue strongly without muting unrelated issues in the project.
            if decayed > 0.0 && ui.kind != UserInteractionKind::ResolveIssue {
                *user_proj_boost
                    .entry(ui.project_slug.to_ascii_lowercase())
                    .or_insert(0.0) += decayed * 0.25;
            }
        }
    }

    let bucket_seed = splitmix64(
        (ctx.now.timestamp().max(0) as u64 / 10)
            ^ ctx.flush_nonce.wrapping_mul(0x9e37_79b9_7f4a_7c15),
    );
    let tau = ctx.config.stochastic_temperature.clamp(0.15, 3.0);
    let theta = ctx.config.relevance_threshold;

    // 4. Score each candidate issue group
    let mut scored_groups: Vec<ScoredCandidateGroup> = Vec::with_capacity(by_issue.len());
    let mut suppressed_count = 0usize;

    for (issue_id, group) in by_issue {
        let first = &group[0];
        let summaries: Vec<&str> = group.iter().map(|c| c.summary.as_str()).collect();
        let combined_summary = CompactString::from(deduplicate_summaries(&summaries));

        let dominant_kind = if group.iter().any(|c| c.kind == DomainChangeKind::NewIssue) {
            DomainChangeKind::NewIssue
        } else if group.iter().any(|c| c.kind == DomainChangeKind::Regressed) {
            DomainChangeKind::Regressed
        } else {
            DomainChangeKind::EventSpike
        };

        let delta_events: u64 = group.iter().map(|c| c.delta_events).sum();
        let total_events = group
            .iter()
            .map(|c| c.total_events)
            .max()
            .unwrap_or(first.total_events)
            .max(1);
        let events_24h = group
            .iter()
            .map(|c| c.events_24h)
            .max()
            .unwrap_or(first.events_24h);
        let user_count = group
            .iter()
            .map(|c| c.user_count)
            .max()
            .unwrap_or(first.user_count);
        let is_unhandled = group.iter().any(|c| c.is_unhandled);
        let level = group
            .iter()
            .map(|c| c.level)
            .min()
            .unwrap_or(first.level);
        let priority = group
            .iter()
            .map(|c| c.priority)
            .min()
            .unwrap_or(first.priority);

        let prior_events = total_events.saturating_sub(delta_events);
        let issue_age_hours = (ctx.now - first.first_seen).num_minutes().max(0) as f64 / 60.0;

        // 4a. Base novelty & relative velocity score (suppresses 40,000-event chronic repeaters)
        let kind_score = match dominant_kind {
            DomainChangeKind::NewIssue => 38.0,
            DomainChangeKind::Regressed => {
                // Fresh regression on a low-volume issue scores high (~32);
                // chronic flapping on a 40,000-event background worker decays down toward ~8.5
                let chronic_damp = 1.0 / (1.0 + ((total_events as f64) / 80.0).ln_1p());
                10.0 + 24.0 * chronic_damp
            }
            DomainChangeKind::EventSpike => {
                let rel_ratio =
                    (delta_events as f64) / ((prior_events.max(events_24h) as f64) + 15.0);
                let burst_boost = 28.0 * (rel_ratio / (rel_ratio + 0.35));
                let abs_boost = 2.2 * (delta_events as f64).ln_1p();
                // Chronic repeater damping for issues that already have thousands of events
                let chronic_penalty = if prior_events > 250 && rel_ratio < 0.15 {
                    0.35
                } else {
                    1.0
                };
                (burst_boost + abs_boost) * chronic_penalty
            }
        };

        // 4b. Severity, Priority, Unhandled & User Impact
        let level_score = match level {
            IssueLevel::Fatal => 24.0,
            IssueLevel::Error => 8.0,
            IssueLevel::Warning => 2.0,
            IssueLevel::Info | IssueLevel::Debug => 0.0,
        };
        let prio_score = match priority {
            IssuePriority::High => 10.0,
            IssuePriority::Medium => 4.0,
            IssuePriority::Low | IssuePriority::None => 0.0,
        };
        let unhandled_score = if is_unhandled {
            if prior_events > 200 && dominant_kind == DomainChangeKind::EventSpike {
                // Do not let a chronic unhandled issue get a free pass forever
                4.0
            } else {
                12.0
            }
        } else {
            0.0
        };
        let users_score = if user_count > 1 {
            (3.0 * (user_count as f64).ln_1p()).min(12.0)
        } else {
            0.0
        };
        let freshness_bonus = if issue_age_hours <= 2.0 {
            8.0 * (1.0 - issue_age_hours / 2.0)
        } else {
            0.0
        };

        let raw_signal =
            kind_score + level_score + prio_score + unhandled_score + users_score + freshness_bonus;

        // 4c. User Interaction Hawkes boost (or negative suppression if ignored)
        let proj_key = first.project_slug.to_ascii_lowercase();
        let mut u_issue = user_issue_boost
            .get(issue_id.as_str())
            .copied()
            .unwrap_or(0.0);
        if dominant_kind == DomainChangeKind::Regressed {
            u_issue += user_issue_resolve_boost
                .get(issue_id.as_str())
                .copied()
                .unwrap_or(0.0);
        }
        let u_proj = if latest_issue_status.get(issue_id.as_str())
            == Some(&UserInteractionKind::IgnoreIssue)
        {
            0.0
        } else {
            user_proj_boost.get(&proj_key).copied().unwrap_or(0.0)
        };
        let user_boost = (u_issue + u_proj).clamp(-50.0, 45.0);

        // 4d. Hawkes Self-Inhibition Fatigue (per-issue + per-project)
        let issue_fatigue = issue_fatigue_map
            .get(issue_id.as_str())
            .copied()
            .unwrap_or(0.0);
        let project_fatigue = proj_fatigue_map
            .get(&proj_key)
            .copied()
            .unwrap_or(0.0)
            .min(40.0);
        let issue_notifs_1h = issue_notifs_1h_map
            .get(issue_id.as_str())
            .copied()
            .unwrap_or(0);

        // Brand-new issues or fatal issues suffer less cross-issue project fatigue so a noisy app
        // doesn't completely hide a genuinely brand-new crash in the same app.
        let effective_proj_fatigue = if dominant_kind == DomainChangeKind::NewIssue
            || level == IssueLevel::Fatal
        {
            project_fatigue * 0.45
        } else {
            project_fatigue
        };

        let net_score = raw_signal + user_boost - issue_fatigue - effective_proj_fatigue;

        // 4e. Logistic emission probability & Gumbel-Top-k key
        let scale = (6.0 * tau).max(1.0);
        let z = ((net_score - theta) / scale).clamp(-15.0, 15.0);
        let emission_probability = 1.0 / (1.0 + (-z).exp());

        let u_gate = seeded_uniform(bucket_seed, issue_id.as_str(), 0xA11CE);
        let gumbel_noise = seeded_gumbel(bucket_seed, issue_id.as_str(), 0xB0B);
        // Keep Gumbel-Top-k on the same linear score scale as the logistic threshold gate
        let gumbel_key = net_score + scale * gumbel_noise;

        let is_pinned_urgent = match effective_mode {
            NotificationFilterMode::Hybrid => {
                issue_notifs_1h == 0
                    && issue_fatigue < 10.0
                    && ((dominant_kind == DomainChangeKind::NewIssue
                        && (is_unhandled || level == IssueLevel::Fatal)
                        && project_fatigue < 35.0)
                        || (level == IssueLevel::Fatal && net_score >= theta)
                        || (dominant_kind == DomainChangeKind::Regressed
                            && total_events <= 200
                            && net_score >= theta))
            }
            NotificationFilterMode::Stochastic
            | NotificationFilterMode::Deterministic
            | NotificationFilterMode::Legacy => false,
        };

        let passed_gate = match effective_mode {
            NotificationFilterMode::Deterministic => net_score >= theta,
            NotificationFilterMode::Hybrid => is_pinned_urgent || (u_gate < emission_probability),
            NotificationFilterMode::Stochastic => u_gate < emission_probability,
            NotificationFilterMode::Legacy => true,
        };

        // 4f. Dynamic Urgency Downgrading:
        // Only use Critical urgency if `critical_unhandled_alert` is enabled AND this issue
        // hasn't already spammed in the last hour AND isn't a chronic >500-event repeater.
        let can_be_critical = ctx.config.critical_unhandled_alert
            && (is_unhandled || level == IssueLevel::Fatal || dominant_kind == DomainChangeKind::Regressed)
            && issue_notifs_1h == 0
            && issue_fatigue < 12.0
            && (dominant_kind == DomainChangeKind::NewIssue
                || level == IssueLevel::Fatal
                || total_events <= 500)
            && net_score >= theta + 6.0;

        let urgency = if can_be_critical {
            NotifyUrgency::Critical
        } else if net_score < theta + 2.0 || project_fatigue >= 25.0 {
            NotifyUrgency::Low
        } else {
            NotifyUrgency::Normal
        };

        if !passed_gate {
            suppressed_count += 1;
        }

        scored_groups.push(ScoredCandidateGroup {
            issue_id,
            short_id: first.short_id.clone(),
            project_slug: first.project_slug.clone(),
            issue_title: first.issue_title.clone(),
            url: first.url.clone(),
            combined_summary,
            dominant_kind,
            level,
            priority,
            is_unhandled,
            delta_events,
            total_events,
            events_24h,
            user_count,
            raw_signal,
            user_boost,
            issue_fatigue,
            project_fatigue,
            issue_notifs_1h,
            net_score,
            emission_probability,
            gumbel_key,
            passed_gate,
            is_pinned_urgent,
            urgency,
        });
    }

    // 5. Filter to candidates that passed the relevance/stochastic gate
    let mut survivors: Vec<ScoredCandidateGroup> = scored_groups
        .into_iter()
        .filter(|g| g.passed_gate)
        .collect();

    if survivors.is_empty() {
        return NotificationEvaluationResult {
            emitted: Vec::new(),
            recorded_emissions: Vec::new(),
            suppressed_count,
        };
    }

    // 6. Order survivors: Critical / pinned-urgent items always lead so coalescing never buries
    // a critical alert behind a normal one, then break ties by mode-specific score.
    match effective_mode {
        NotificationFilterMode::Deterministic => {
            survivors.sort_by(|a, b| {
                let a_crit = a.is_pinned_urgent || a.urgency == NotifyUrgency::Critical;
                let b_crit = b.is_pinned_urgent || b.urgency == NotifyUrgency::Critical;
                b_crit.cmp(&a_crit).then_with(|| {
                    b.net_score
                        .partial_cmp(&a.net_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });
        }
        NotificationFilterMode::Stochastic
        | NotificationFilterMode::Hybrid
        | NotificationFilterMode::Legacy => {
            survivors.sort_by(|a, b| {
                let a_crit = a.is_pinned_urgent || a.urgency == NotifyUrgency::Critical;
                let b_crit = b.is_pinned_urgent || b.urgency == NotifyUrgency::Critical;
                b_crit
                    .cmp(&a_crit)
                    .then_with(|| b.is_pinned_urgent.cmp(&a.is_pinned_urgent))
                    .then_with(|| {
                        b.gumbel_key
                            .partial_cmp(&a.gumbel_key)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
        }
    }

    // 7. Enforce windowed notification budget (`max_notifications_per_flush`) and coalesce overflow
    let max_budget = ctx.config.max_notifications_per_flush.max(1);
    let coalesce_threshold = ctx.config.coalesce_threshold.max(1);

    let mut emitted = Vec::new();
    let mut recorded_emissions = Vec::new();
    let base_ts_ms = ctx.now.timestamp_millis().max(1) as u64;

    // Record fatigue for all survivors that actually contribute to a notification so a coalesced burst
    // also cools down the noisy project and its constituent issues.
    for (idx, surv) in survivors.iter().enumerate() {
        recorded_emissions.push(NotificationEmission {
            id: base_ts_ms
                .wrapping_mul(100)
                .wrapping_add((idx as u64) % 97),
            issue_id: surv.issue_id.clone(),
            short_id: surv.short_id.clone(),
            project_slug: surv.project_slug.clone(),
            urgency: CompactString::from(surv.urgency.as_str()),
            score: surv.net_score,
            created_at: ctx.now,
        });
    }

    if survivors.len() > coalesce_threshold || survivors.len() > max_budget {
        // Emit top pinned/critical winner(s) individually up to `max_budget - 1`, and coalesce the rest
        // into a single compact digest notification (or 1 single digest if max_budget == 1).
        let individual_slots = if survivors.len() > coalesce_threshold {
            survivors
                .iter()
                .take(max_budget.saturating_sub(1))
                .filter(|s| s.urgency == NotifyUrgency::Critical || s.is_pinned_urgent)
                .count()
        } else {
            max_budget.saturating_sub(1)
        };

        let mut iter = survivors.into_iter();
        for _ in 0..individual_slots {
            if let Some(s) = iter.next() {
                emitted.push(format_single_candidate_notification(&s));
            }
        }

        let remaining: Vec<ScoredCandidateGroup> = iter.collect();
        if remaining.len() == 1 {
            let s = &remaining[0];
            emitted.push(format_single_candidate_notification(s));
        } else if !remaining.is_empty() {
            let issue_count = remaining.len();
            let projects: BTreeSet<&str> =
                remaining.iter().map(|s| s.project_slug.as_str()).collect();
            let total_delta: u64 = remaining.iter().map(|s| s.delta_events).sum();
            let proj_label = if projects.is_empty() {
                "Sentry".to_string()
            } else {
                projects.into_iter().take(4).collect::<Vec<_>>().join(", ")
            };
            let summary = if total_delta > 0 {
                format!("{issue_count} relevant Sentry issues in {proj_label} (+{total_delta} events)")
            } else {
                format!("{issue_count} relevant Sentry issues updated in {proj_label}")
            };
            let preview_limit = 6usize;
            let mut identifiers: Vec<String> = remaining
                .iter()
                .take(preview_limit)
                .map(|s| match s.dominant_kind {
                    DomainChangeKind::NewIssue => format!("{} (new)", s.short_id),
                    DomainChangeKind::Regressed => format!("{} (regressed)", s.short_id),
                    DomainChangeKind::EventSpike => {
                        if s.delta_events > 0 {
                            format!("{} (+{})", s.short_id, s.delta_events)
                        } else {
                            s.short_id.to_string()
                        }
                    }
                })
                .collect();
            if remaining.len() > preview_limit {
                identifiers.push(format!("+{} more", remaining.len() - preview_limit));
            }
            let top_url = remaining.first().map(|s| s.url.clone());

            emitted.push(EmittedNotification {
                summary: summary.into(),
                body: format!("Issues: {}", identifiers.join(", ")).into(),
                urgency: NotifyUrgency::Normal,
                url: top_url,
            });
        }
    } else {
        for s in &survivors {
            emitted.push(format_single_candidate_notification(s));
        }
    }

    NotificationEvaluationResult {
        emitted,
        recorded_emissions,
        suppressed_count,
    }
}

fn format_single_candidate_notification(s: &ScoredCandidateGroup) -> EmittedNotification {
    let badge = s.dominant_kind.badge();
    let summary = if s.urgency == NotifyUrgency::Critical {
        format!(
            "[{} • {} • {}] {} — {}",
            s.project_slug,
            s.level.label(),
            badge,
            s.short_id,
            s.issue_title
        )
    } else {
        format!(
            "[{} • {}] {}: {}",
            s.project_slug, badge, s.short_id, s.issue_title
        )
    };
    let body = if s.user_count > 1 {
        CompactString::from(format!(
            "{} • {} users affected",
            s.combined_summary, s.user_count
        ))
    } else {
        s.combined_summary.clone()
    };
    EmittedNotification {
        summary: summary.into(),
        body,
        urgency: s.urgency,
        url: Some(s.url.clone()),
    }
}

fn evaluate_legacy_notifications(
    changes: Vec<DomainChange>,
    config: &NotificationsSection,
    now: DateTime<Utc>,
) -> NotificationEvaluationResult {
    let mut by_issue: BTreeMap<IssueId, Vec<DomainChange>> = BTreeMap::new();
    for change in changes {
        by_issue
            .entry(change.issue_id.clone())
            .or_default()
            .push(change);
    }

    let mut emitted = Vec::new();
    let mut recorded_emissions = Vec::new();
    let mut standard_groups: Vec<(IssueId, Vec<DomainChange>)> = Vec::new();
    let base_ts_ms = now.timestamp_millis().max(1) as u64;

    for (idx, (issue_id, group)) in by_issue.into_iter().enumerate() {
        let first = &group[0];
        recorded_emissions.push(NotificationEmission {
            id: base_ts_ms.wrapping_mul(100).wrapping_add((idx as u64) % 97),
            issue_id: issue_id.clone(),
            short_id: first.short_id.clone(),
            project_slug: first.project_slug.clone(),
            urgency: CompactString::from(if first.is_critical {
                "critical"
            } else {
                "normal"
            }),
            score: 100.0,
            created_at: now,
        });

        let is_critical =
            config.critical_unhandled_alert && group.iter().any(|c| c.is_critical);

        if is_critical {
            let summaries: Vec<&str> = group.iter().map(|c| c.summary.as_str()).collect();
            let combined_summary = deduplicate_summaries(&summaries);
            emitted.push(EmittedNotification {
                summary: format!(
                    "[{} • {}] {} — {}",
                    first.project_slug,
                    first.level.label(),
                    first.short_id,
                    first.issue_title
                )
                .into(),
                body: combined_summary.into(),
                urgency: NotifyUrgency::Critical,
                url: Some(first.url.clone()),
            });
        } else {
            standard_groups.push((issue_id, group));
        }
    }

    let coalesce_threshold = config.coalesce_threshold.max(1);
    if standard_groups.len() > coalesce_threshold {
        let issue_count = standard_groups.len();
        let projects: BTreeSet<&str> = standard_groups
            .iter()
            .filter_map(|(_, g)| g.first().map(|c| c.project_slug.as_str()))
            .collect();
        let total_delta: u64 = standard_groups
            .iter()
            .flat_map(|(_, g)| g.iter().map(|c| c.delta_events))
            .sum();

        let proj_label = if projects.is_empty() {
            "Sentry".to_string()
        } else {
            projects.into_iter().take(4).collect::<Vec<_>>().join(", ")
        };
        let summary = if total_delta > 0 {
            format!("{issue_count} Sentry issues active in {proj_label} (+{total_delta} events)")
        } else {
            format!("{issue_count} Sentry issues updated in {proj_label}")
        };

        let identifiers: Vec<&str> = standard_groups
            .iter()
            .filter_map(|(_, g)| g.first().map(|c| c.short_id.as_str()))
            .take(8)
            .collect();

        emitted.push(EmittedNotification {
            summary: summary.into(),
            body: format!("Issues: {}", identifiers.join(", ")).into(),
            urgency: NotifyUrgency::Normal,
            url: None,
        });
    } else {
        for (_issue_id, group) in standard_groups {
            if config.per_issue_dedup {
                let first = &group[0];
                let summaries: Vec<&str> = group.iter().map(|c| c.summary.as_str()).collect();
                let combined_summary = deduplicate_summaries(&summaries);
                emitted.push(EmittedNotification {
                    summary: format!(
                        "[{}] {}: {}",
                        first.project_slug, first.short_id, first.issue_title
                    )
                    .into(),
                    body: combined_summary.into(),
                    urgency: NotifyUrgency::Normal,
                    url: Some(first.url.clone()),
                });
            } else {
                for change in group {
                    emitted.push(EmittedNotification {
                        summary: format!(
                            "[{}] {}: {}",
                            change.project_slug, change.short_id, change.issue_title
                        )
                        .into(),
                        body: change.summary,
                        urgency: NotifyUrgency::Normal,
                        url: Some(change.url),
                    });
                }
            }
        }
    }

    NotificationEvaluationResult {
        emitted,
        recorded_emissions,
        suppressed_count: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_change(
        id: &str,
        short_id: &str,
        project: &str,
        kind: DomainChangeKind,
        is_unhandled: bool,
        delta: u64,
        total: u64,
    ) -> DomainChange {
        let now = Utc::now();
        DomainChange {
            issue_id: IssueId::from(id),
            short_id: CompactString::from(short_id),
            project_slug: CompactString::from(project),
            issue_title: CompactString::from("Worker failed"),
            summary: CompactString::from(format!("+{delta} new events (total: {total})")),
            url: CompactString::from(format!("https://sentry.io/issues/{id}/")),
            kind,
            level: IssueLevel::Error,
            priority: IssuePriority::Medium,
            is_unhandled,
            is_critical: is_unhandled || kind == DomainChangeKind::Regressed,
            delta_events: delta,
            total_events: total,
            events_24h: total.min(5000),
            user_count: 1,
            first_seen: now - Duration::days(7),
            last_seen: now,
        }
    }

    #[test]
    fn test_chronic_repeater_and_hawkes_fatigue_suppression() {
        let cfg = NotificationsSection::default();
        let now = Utc::now();

        // Simulate a noisy chronic repeater like IRIS-TERMINAL-CB (40,151 events, +36 delta)
        // that already emitted 1 notification 2 minutes ago.
        let recent_emissions = vec![NotificationEmission {
            id: 1,
            issue_id: IssueId::from("7670877591"),
            short_id: CompactString::from("IRIS-TERMINAL-CB"),
            project_slug: CompactString::from("iris-terminal"),
            urgency: CompactString::from("normal"),
            score: 22.0,
            created_at: now - Duration::minutes(2),
        }];

        let change = sample_change(
            "7670877591",
            "IRIS-TERMINAL-CB",
            "iris-terminal",
            DomainChangeKind::EventSpike,
            true, // unhandled in Sentry!
            36,
            40151,
        );

        let ctx = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &recent_emissions,
            user_interactions: &[],
            filter_mode: NotificationFilterMode::Hybrid,
            now,
            flush_nonce: 1,
        };

        let res = evaluate_notifications(vec![change], &ctx);
        assert!(
            res.emitted.is_empty(),
            "Chronic 40k-event repeater with recent emission should be suppressed by Hawkes self-inhibition"
        );
        assert_eq!(res.suppressed_count, 1);
    }

    #[test]
    fn test_brand_new_unhandled_issue_passes_immediately() {
        let cfg = NotificationsSection::default();
        let now = Utc::now();

        let mut new_issue = sample_change(
            "999999",
            "CAASA-CONNECT-99",
            "caasa-connect",
            DomainChangeKind::NewIssue,
            true,
            2,
            2,
        );
        new_issue.first_seen = now - Duration::minutes(1);

        let ctx = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &[],
            user_interactions: &[],
            filter_mode: NotificationFilterMode::Hybrid,
            now,
            flush_nonce: 1,
        };

        let res = evaluate_notifications(vec![new_issue], &ctx);
        assert_eq!(res.emitted.len(), 1);
        assert_eq!(res.emitted[0].urgency, NotifyUrgency::Critical);
        assert_eq!(res.suppressed_count, 0);
    }

    #[test]
    fn test_user_interaction_boost_elevates_inspected_issue() {
        let cfg = NotificationsSection {
            filter_mode: NotificationFilterMode::Deterministic,
            ..NotificationsSection::default()
        };
        let now = Utc::now();

        // Moderate spike on an issue that already emitted 30 mins ago (normally suppressed)
        let recent_emissions = vec![NotificationEmission {
            id: 1,
            issue_id: IssueId::from("12345"),
            short_id: CompactString::from("APP-12"),
            project_slug: CompactString::from("my-app"),
            urgency: CompactString::from("normal"),
            score: 20.0,
            created_at: now - Duration::minutes(30),
        }];

        let change = sample_change(
            "12345",
            "APP-12",
            "my-app",
            DomainChangeKind::EventSpike,
            false,
            6,
            120,
        );

        // Without user interaction: suppressed
        let ctx_no_inter = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &recent_emissions,
            user_interactions: &[],
            filter_mode: NotificationFilterMode::Deterministic,
            now,
            flush_nonce: 1,
        };
        let res_no = evaluate_notifications(vec![change.clone()], &ctx_no_inter);
        assert!(res_no.emitted.is_empty());

        // With recent user inspection + open in web: boosted above threshold!
        let interactions = vec![
            UserInteraction {
                id: 10,
                issue_id: IssueId::from("12345"),
                short_id: CompactString::from("APP-12"),
                project_slug: CompactString::from("my-app"),
                kind: crate::domain::UserInteractionKind::OpenIssueWeb,
                weight: crate::domain::UserInteractionKind::OpenIssueWeb.base_weight(),
                created_at: now - Duration::minutes(5),
            },
            UserInteraction {
                id: 11,
                issue_id: IssueId::from("12345"),
                short_id: CompactString::from("APP-12"),
                project_slug: CompactString::from("my-app"),
                kind: crate::domain::UserInteractionKind::InspectIssueDetail,
                weight: crate::domain::UserInteractionKind::InspectIssueDetail.base_weight(),
                created_at: now - Duration::minutes(5),
            },
        ];
        let ctx_with_inter = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &recent_emissions,
            user_interactions: &interactions,
            filter_mode: NotificationFilterMode::Deterministic,
            now,
            flush_nonce: 1,
        };
        let res_yes = evaluate_notifications(vec![change], &ctx_with_inter);
        assert_eq!(res_yes.emitted.len(), 1);
    }

    #[test]
    fn test_critical_first_coalescing_preserves_critical_alert() {
        let cfg = NotificationsSection {
            filter_mode: NotificationFilterMode::Deterministic,
            coalesce_threshold: 2,
            max_notifications_per_flush: 2,
            ..NotificationsSection::default()
        };
        let now = Utc::now();

        // Two normal spikes with high user boost vs one critical brand-new unhandled issue
        let normal_a = sample_change(
            "1001",
            "APP-101",
            "my-app",
            DomainChangeKind::EventSpike,
            false,
            25,
            30,
        );
        let normal_b = sample_change(
            "1002",
            "APP-102",
            "my-app",
            DomainChangeKind::EventSpike,
            false,
            22,
            28,
        );
        let mut crit_new = sample_change(
            "1003",
            "APP-999",
            "my-app",
            DomainChangeKind::NewIssue,
            true,
            1,
            1,
        );
        crit_new.level = IssueLevel::Fatal;
        crit_new.first_seen = now - Duration::minutes(1);

        // Give normal_a a massive user boost so its raw net_score exceeds crit_new,
        // verifying that Critical urgency still sorts ahead of Normal during coalescing!
        let interactions = vec![UserInteraction {
            id: 1,
            issue_id: IssueId::from("1001"),
            short_id: CompactString::from("APP-101"),
            project_slug: CompactString::from("my-app"),
            kind: UserInteractionKind::OpenIssueWeb,
            weight: 45.0,
            created_at: now - Duration::minutes(1),
        }];

        let ctx = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &[],
            user_interactions: &interactions,
            filter_mode: NotificationFilterMode::Deterministic,
            now,
            flush_nonce: 1,
        };

        let res = evaluate_notifications(vec![normal_a, normal_b, crit_new], &ctx);
        assert_eq!(res.emitted.len(), 2);
        assert_eq!(res.emitted[0].urgency, NotifyUrgency::Critical);
        assert!(res.emitted[0].summary.contains("APP-999"));
        assert!(res.emitted[0].summary.contains("NEW"));
        assert_eq!(res.emitted[1].urgency, NotifyUrgency::Normal);
        assert!(res.emitted[1].body.contains("APP-101 (+25)"));
        assert!(res.emitted[1].body.contains("APP-102 (+22)"));
    }

    #[test]
    fn test_resolve_interaction_only_boosts_regressions_and_buffer_purges_resolved_issue() {
        let cfg = NotificationsSection {
            filter_mode: NotificationFilterMode::Deterministic,
            ..NotificationsSection::default()
        };
        let now = Utc::now();

        let recent_emissions = vec![NotificationEmission {
            id: 1,
            issue_id: IssueId::from("5555"),
            short_id: CompactString::from("API-55"),
            project_slug: CompactString::from("api"),
            urgency: CompactString::from("normal"),
            score: 20.0,
            created_at: now - Duration::minutes(20),
        }];

        let resolve_interaction = vec![UserInteraction {
            id: 2,
            issue_id: IssueId::from("5555"),
            short_id: CompactString::from("API-55"),
            project_slug: CompactString::from("api"),
            kind: UserInteractionKind::ResolveIssue,
            weight: 30.0,
            created_at: now - Duration::minutes(5),
        }];

        // 1. A routine post-resolution straggler spike should NOT get boosted by ResolveIssue
        let spike = sample_change(
            "5555",
            "API-55",
            "api",
            DomainChangeKind::EventSpike,
            false,
            4,
            100,
        );
        let ctx = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &recent_emissions,
            user_interactions: &resolve_interaction,
            filter_mode: NotificationFilterMode::Deterministic,
            now,
            flush_nonce: 1,
        };
        let res_spike = evaluate_notifications(vec![spike.clone()], &ctx);
        assert!(
            res_spike.emitted.is_empty(),
            "ResolveIssue should not boost non-regression EventSpike stragglers"
        );

        // 2. A true regression on that resolved issue SHOULD receive the ResolveIssue boost
        let regressed = sample_change(
            "5555",
            "API-55",
            "api",
            DomainChangeKind::Regressed,
            false,
            4,
            100,
        );
        let res_reg = evaluate_notifications(vec![regressed], &ctx);
        assert_eq!(
            res_reg.emitted.len(),
            1,
            "ResolveIssue should boost subsequent regressions"
        );
        assert!(res_reg.emitted[0].summary.contains("REGRESSED"));

        // 3. NotificationBuffer::drop_issue purges queued changes for resolved/ignored issues
        let mut buf = crate::notification_buffer::NotificationBuffer::new();
        buf.enqueue(vec![spike]);
        buf.drop_issue(&IssueId::from("5555"));
        let flushed = buf.evaluate_and_flush(
            &cfg,
            &[],
            &[],
            NotificationFilterMode::Deterministic,
            false,
        );
        assert!(flushed.emitted.is_empty());
    }

    #[test]
    fn test_status_barrier_prevents_inspect_before_ignore_or_stale_ignore_after_unresolve() {
        let cfg = NotificationsSection {
            filter_mode: NotificationFilterMode::Deterministic,
            ..NotificationsSection::default()
        };
        let now = Utc::now();

        let spike = sample_change(
            "7777",
            "WEB-77",
            "web",
            DomainChangeKind::EventSpike,
            false,
            15,
            30,
        );

        // Case A: User inspected (+16) and opened in web (+24) BEFORE ignoring (-45).
        // Because interactions are ordered newest-first, IgnoreIssue is at index 0.
        let interactions_ignored = vec![
            UserInteraction {
                id: 3,
                issue_id: IssueId::from("7777"),
                short_id: CompactString::from("WEB-77"),
                project_slug: CompactString::from("web"),
                kind: UserInteractionKind::IgnoreIssue,
                weight: UserInteractionKind::IgnoreIssue.base_weight(),
                created_at: now - Duration::minutes(1),
            },
            UserInteraction {
                id: 2,
                issue_id: IssueId::from("7777"),
                short_id: CompactString::from("WEB-77"),
                project_slug: CompactString::from("web"),
                kind: UserInteractionKind::OpenIssueWeb,
                weight: UserInteractionKind::OpenIssueWeb.base_weight(),
                created_at: now - Duration::minutes(2),
            },
            UserInteraction {
                id: 1,
                issue_id: IssueId::from("7777"),
                short_id: CompactString::from("WEB-77"),
                project_slug: CompactString::from("web"),
                kind: UserInteractionKind::InspectIssueDetail,
                weight: UserInteractionKind::InspectIssueDetail.base_weight(),
                created_at: now - Duration::minutes(3),
            },
        ];

        let ctx_ignored = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &[],
            user_interactions: &interactions_ignored,
            filter_mode: NotificationFilterMode::Deterministic,
            now,
            flush_nonce: 1,
        };
        let res_ignored = evaluate_notifications(vec![spike.clone()], &ctx_ignored);
        assert!(
            res_ignored.emitted.is_empty(),
            "Prior inspect/open interactions must not cancel out a newer IgnoreIssue action"
        );

        // Case B: User later Unresolves (+26) the previously ignored issue (newest-first).
        let mut interactions_unresolved = vec![UserInteraction {
            id: 4,
            issue_id: IssueId::from("7777"),
            short_id: CompactString::from("WEB-77"),
            project_slug: CompactString::from("web"),
            kind: UserInteractionKind::UnresolveIssue,
            weight: UserInteractionKind::UnresolveIssue.base_weight(),
            created_at: now,
        }];
        interactions_unresolved.extend(interactions_ignored);

        let ctx_unresolved = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &[],
            user_interactions: &interactions_unresolved,
            filter_mode: NotificationFilterMode::Deterministic,
            now,
            flush_nonce: 1,
        };
        let res_unresolved = evaluate_notifications(vec![spike], &ctx_unresolved);
        assert_eq!(
            res_unresolved.emitted.len(),
            1,
            "Unresolving an issue must clear older IgnoreIssue penalties"
        );
    }

    #[test]
    fn test_project_fatigue_cap_preserves_new_fatal_alert_after_coalesced_burst() {
        let cfg = NotificationsSection::default();
        let now = Utc::now();

        // Simulate 15 recent emissions in the same project from a coalesced burst 1 minute ago
        let recent_emissions: Vec<NotificationEmission> = (0..15)
            .map(|i| NotificationEmission {
                id: i + 1,
                issue_id: IssueId::from(format!("burst-{i}")),
                short_id: CompactString::from(format!("BURST-{i}")),
                project_slug: CompactString::from("noisy-service"),
                urgency: CompactString::from("normal"),
                score: 25.0,
                created_at: now - Duration::minutes(1),
            })
            .collect();

        // Now a brand-new Fatal crash arrives in `noisy-service`
        let mut fatal_new = sample_change(
            "fatal-999",
            "NOISY-FATAL-1",
            "noisy-service",
            DomainChangeKind::NewIssue,
            true,
            1,
            1,
        );
        fatal_new.level = IssueLevel::Fatal;
        fatal_new.first_seen = now;

        let ctx = NotificationRelevanceContext {
            config: &cfg,
            recent_emissions: &recent_emissions,
            user_interactions: &[],
            filter_mode: NotificationFilterMode::Hybrid,
            now,
            flush_nonce: 1,
        };

        let res = evaluate_notifications(vec![fatal_new], &ctx);
        assert_eq!(
            res.emitted.len(),
            1,
            "Capped project_fatigue must not mute a brand-new Fatal crash after a coalesced burst"
        );
        assert_eq!(res.emitted[0].urgency, NotifyUrgency::Critical);
    }
}
