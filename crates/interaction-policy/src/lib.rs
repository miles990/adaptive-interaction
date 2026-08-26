//! Deterministic policy / consent / safety governor.
//!
//! Every side effect passes through [`Governor::authorize`]. The governor is a
//! pure function over its inputs — no clocks, no I/O — so it is fully testable
//! and cannot be bypassed by prompts. The effective output limit is always:
//!
//! ```text
//! effective = min(AI suggestion, user preference (policy), session limit,
//!                 device safe limit, remaining accumulated budget)
//! ```

use chrono::NaiveTime;
use interaction_core::{
    ActionParameters, ActuatorManifest, AuthorizationOutcome, ConsentScope, PatternSpec,
    PolicyConfig, PolicyDecision, QuietHours, RiskClass, Session, Timestamp,
};

/// Where an interaction request originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionSource {
    /// A human or AI explicitly asked for this action in-session.
    ExplicitRequest,
    /// The runtime initiated it autonomously (recipe trigger, orchestrator).
    Autonomous,
}

/// Usage counters supplied by storage; the governor stays pure.
#[derive(Debug, Clone, Default)]
pub struct UsageContext {
    /// Completed/accepted invocations of this actuator in the last rolling hour.
    pub actuator_fired_last_hour: u32,
    /// Last time this actuator fired (for cooldown).
    pub actuator_last_fired_at: Option<Timestamp>,
    /// Active-duration already consumed on this channel this session (ms).
    pub channel_budget_used_ms: u64,
    /// Money spent this session (USD).
    pub monetary_spent: f64,
    /// Actions currently queued or running runtime-wide.
    pub scheduled_actions: u32,
}

/// Full authorization input.
pub struct AuthorizationRequest<'a> {
    pub actuator: &'a ActuatorManifest,
    pub requested: &'a ActionParameters,
    pub intent: &'a str,
    pub source: ActionSource,
    /// Local wall-clock time for quiet-hours evaluation (injectable for tests).
    pub local_time: NaiveTime,
    pub now: Timestamp,
    pub emergency_stop_engaged: bool,
}

#[derive(Debug, Clone)]
pub struct AuthorizationResult {
    pub outcome: AuthorizationOutcome,
    pub decisions: Vec<PolicyDecision>,
    /// Only meaningful when `outcome == Authorized`.
    pub effective: ActionParameters,
}

impl AuthorizationResult {
    fn blocked(rule: &str, reason: String, mut decisions: Vec<PolicyDecision>) -> Self {
        decisions.push(PolicyDecision::Blocked {
            rule: rule.to_string(),
            reason,
        });
        Self {
            outcome: AuthorizationOutcome::Blocked,
            decisions,
            effective: ActionParameters::default(),
        }
    }

    fn approval(rule: &str, reason: String, mut decisions: Vec<PolicyDecision>) -> Self {
        decisions.push(PolicyDecision::ApprovalRequired {
            rule: rule.to_string(),
            reason,
        });
        Self {
            outcome: AuthorizationOutcome::ApprovalRequired,
            decisions,
            effective: ActionParameters::default(),
        }
    }
}

/// Channels considered intrusive during quiet hours when a window does not
/// list explicit channels.
const DEFAULT_QUIET_SILENCED: &[&str] =
    &["audio", "haptic", "notification", "light", "desktop-pet"];

pub struct Governor;

impl Governor {
    /// Authorize one planned step against policy + session + usage.
    pub fn authorize(
        policy: &PolicyConfig,
        session: &Session,
        req: &AuthorizationRequest<'_>,
        usage: &UsageContext,
    ) -> AuthorizationResult {
        let mut decisions: Vec<PolicyDecision> = Vec::new();
        let actuator = req.actuator;
        let channel = actuator.channel.as_str();

        // 0. Emergency stop dominates everything.
        if req.emergency_stop_engaged {
            return AuthorizationResult::blocked(
                "emergency-stop",
                "emergency stop is engaged; no actuation allowed".into(),
                decisions,
            );
        }

        // 1. Master switch.
        if !policy.enabled {
            return AuthorizationResult::blocked(
                "policy.enabled",
                "policy master switch is off".into(),
                decisions,
            );
        }

        // 2. Session must be active.
        if !session.is_active(req.now) {
            return AuthorizationResult::blocked(
                "session.active",
                format!("session {} is not active", session.session_id),
                decisions,
            );
        }

        // 3. Actuator allowlist.
        if !list_matches(&policy.actuator_allowlist, actuator.id.as_str()) {
            return AuthorizationResult::blocked(
                "actuator.allowlist",
                format!("actuator {} is not on the allowlist", actuator.id),
                decisions,
            );
        }

        // 4. Channel allowlist.
        if !policy.allowed_channels.is_empty() && !list_matches(&policy.allowed_channels, channel) {
            return AuthorizationResult::blocked(
                "channel.allowlist",
                format!("channel {channel} is not allowed"),
                decisions,
            );
        }

        // 5. Initiative gating for autonomous actions.
        if req.source == ActionSource::Autonomous {
            match policy.initiative {
                interaction_core::InitiativeLevel::Passive => {
                    return AuthorizationResult::blocked(
                        "initiative.passive",
                        "initiative=passive forbids autonomous actions".into(),
                        decisions,
                    );
                }
                interaction_core::InitiativeLevel::Suggest => {
                    if actuator.risk_class > RiskClass::Low || actuator.external_side_effect {
                        return AuthorizationResult::blocked(
                            "initiative.suggest",
                            "initiative=suggest only allows low-risk local channels autonomously"
                                .into(),
                            decisions,
                        );
                    }
                }
                interaction_core::InitiativeLevel::Active => {}
            }
        }

        // 6. Consent for consent-gated actuators / channels.
        if actuator.requires_consent {
            let by_actuator = session.has_consent(
                &ConsentScope::Actuator(actuator.id.as_str().to_string()),
                req.now,
            );
            let by_channel =
                session.has_consent(&ConsentScope::Channel(channel.to_string()), req.now);
            if !by_actuator && !by_channel {
                return AuthorizationResult::blocked(
                    "consent.required",
                    format!("actuator {} requires session consent", actuator.id),
                    decisions,
                );
            }
            decisions.push(PolicyDecision::Allowed {
                rule: "consent.granted".into(),
            });
        }

        // 7. High-risk approval gate. An explicit short-lived actuator consent
        //    granted by a human counts as the approval.
        if actuator.risk_class >= policy.require_approval_at {
            let approved = session.has_consent(
                &ConsentScope::Actuator(actuator.id.as_str().to_string()),
                req.now,
            );
            if !approved {
                return AuthorizationResult::approval(
                    "risk.approval",
                    format!(
                        "risk class {:?} requires explicit human approval",
                        actuator.risk_class
                    ),
                    decisions,
                );
            }
            decisions.push(PolicyDecision::Allowed {
                rule: "risk.approved".into(),
            });
        }

        // 8. Quiet hours.
        if let Some(window) = active_quiet_window(&policy.quiet_hours, req.local_time) {
            let silenced: Vec<&str> = if window.silenced_channels.is_empty() {
                DEFAULT_QUIET_SILENCED.to_vec()
            } else {
                window
                    .silenced_channels
                    .iter()
                    .map(|s| s.as_str())
                    .collect()
            };
            if silenced.contains(&channel) {
                return AuthorizationResult::blocked(
                    "quiet-hours",
                    format!(
                        "channel {channel} is silenced during quiet hours {}-{}",
                        window.start, window.end
                    ),
                    decisions,
                );
            }
            decisions.push(PolicyDecision::Allowed {
                rule: "quiet-hours.non-silenced".into(),
            });
        }

        // 9. Scheduling pressure.
        if usage.scheduled_actions >= policy.max_scheduled_actions {
            return AuthorizationResult::blocked(
                "scheduler.capacity",
                format!(
                    "scheduled action limit {} reached",
                    policy.max_scheduled_actions
                ),
                decisions,
            );
        }

        // 10. Frequency / cooldown.
        let channel_limits = merged_channel_limits(policy, channel);
        let max_per_hour = min_opt_u32(channel_limits.max_per_hour, actuator.limits.max_per_hour);
        if let Some(cap) = max_per_hour {
            if usage.actuator_fired_last_hour >= cap {
                return AuthorizationResult::blocked(
                    "frequency.hourly",
                    format!("hourly limit {cap} reached for {}", actuator.id),
                    decisions,
                );
            }
        }
        if let (Some(cooldown), Some(last)) =
            (channel_limits.cooldown_ms, usage.actuator_last_fired_at)
        {
            let elapsed = req.now.signed_duration_since(last).num_milliseconds();
            if elapsed >= 0 && (elapsed as u64) < cooldown {
                return AuthorizationResult::blocked(
                    "cooldown",
                    format!(
                        "cooldown {}ms not elapsed for {} ({}ms since last)",
                        cooldown, actuator.id, elapsed
                    ),
                    decisions,
                );
            }
        }

        // 11. Monetary budget.
        let invocation_cost = actuator.cost.monetary_per_invocation;
        if invocation_cost > 0.0 {
            let remaining = policy.session_monetary_budget - usage.monetary_spent;
            if invocation_cost > remaining {
                return AuthorizationResult::blocked(
                    "budget.monetary",
                    format!(
                        "invocation cost {invocation_cost} exceeds remaining budget {remaining}"
                    ),
                    decisions,
                );
            }
        }

        // 12. Magnitude bounding: min(requested, policy, device safe limit).
        let mut effective = req.requested.clone();
        if let Some(requested_mag) = effective.magnitude {
            let mut mag = requested_mag.clamp(0.0, 1.0);
            if (mag - requested_mag).abs() > f64::EPSILON {
                decisions.push(PolicyDecision::Clamped {
                    rule: "magnitude.normalize".into(),
                    field: "magnitude".into(),
                    from: requested_mag,
                    to: mag,
                });
            }
            for (rule, cap) in [
                ("magnitude.policy", channel_limits.max_magnitude),
                ("magnitude.device", actuator.limits.max_magnitude),
            ] {
                if let Some(cap) = cap {
                    if mag > cap {
                        decisions.push(PolicyDecision::Clamped {
                            rule: rule.into(),
                            field: "magnitude".into(),
                            from: mag,
                            to: cap,
                        });
                        mag = cap;
                    }
                }
            }
            effective.magnitude = Some(mag);
        }

        // 13. Duration bounding incl. remaining session budget.
        if let Some(requested_dur) = effective.duration_ms {
            let mut dur = requested_dur;
            for (rule, cap) in [
                ("duration.policy", channel_limits.max_duration_ms),
                ("duration.device", actuator.limits.max_duration_ms),
            ] {
                if let Some(cap) = cap {
                    if dur > cap {
                        decisions.push(PolicyDecision::Clamped {
                            rule: rule.into(),
                            field: "durationMs".into(),
                            from: dur as f64,
                            to: cap as f64,
                        });
                        dur = cap;
                    }
                }
            }
            if let Some(budget) = channel_limits.session_budget_ms {
                let remaining = budget.saturating_sub(usage.channel_budget_used_ms);
                if remaining == 0 {
                    return AuthorizationResult::blocked(
                        "budget.channel",
                        format!("session budget for channel {channel} exhausted"),
                        decisions,
                    );
                }
                if dur > remaining {
                    decisions.push(PolicyDecision::Clamped {
                        rule: "budget.channel".into(),
                        field: "durationMs".into(),
                        from: dur as f64,
                        to: remaining as f64,
                    });
                    dur = remaining;
                }
            }
            effective.duration_ms = Some(dur);
        }

        // 14. Payload size limit.
        if let Some(max_bytes) = actuator.limits.max_payload_bytes {
            let size = effective
                .extra
                .as_ref()
                .map(|v| {
                    serde_json::to_vec(v)
                        .map(|b| b.len() as u64)
                        .unwrap_or(u64::MAX)
                })
                .unwrap_or(0)
                + effective
                    .message
                    .as_ref()
                    .map(|m| m.len() as u64)
                    .unwrap_or(0);
            if size > max_bytes {
                return AuthorizationResult::blocked(
                    "payload.size",
                    format!("payload {size} bytes exceeds limit {max_bytes}"),
                    decisions,
                );
            }
        }

        decisions.push(PolicyDecision::Allowed {
            rule: "authorized".into(),
        });
        AuthorizationResult {
            outcome: AuthorizationOutcome::Authorized,
            decisions,
            effective,
        }
    }

    /// Bound a pattern: clamp step count, magnitudes, chance and jitter.
    /// `max_magnitude` should be the already-computed effective magnitude cap.
    pub fn bound_pattern(
        policy: &PolicyConfig,
        actuator: &ActuatorManifest,
        pattern: &PatternSpec,
        max_magnitude: f64,
    ) -> (PatternSpec, Vec<PolicyDecision>) {
        let mut decisions = Vec::new();
        let mut bounded = pattern.clone();

        let step_cap = actuator
            .limits
            .max_pattern_steps
            .map(|d| d.min(policy.max_pattern_steps))
            .unwrap_or(policy.max_pattern_steps) as usize;
        if bounded.steps.len() > step_cap {
            decisions.push(PolicyDecision::Clamped {
                rule: "pattern.steps".into(),
                field: "steps".into(),
                from: bounded.steps.len() as f64,
                to: step_cap as f64,
            });
            bounded.steps.truncate(step_cap);
        }
        for step in bounded.steps.iter_mut() {
            if step.magnitude > max_magnitude {
                decisions.push(PolicyDecision::Clamped {
                    rule: "pattern.magnitude".into(),
                    field: "steps[].magnitude".into(),
                    from: step.magnitude,
                    to: max_magnitude,
                });
                step.magnitude = max_magnitude;
            }
        }
        let chance = bounded.chance.clamp(0.0, 1.0);
        if (chance - bounded.chance).abs() > f64::EPSILON {
            decisions.push(PolicyDecision::Clamped {
                rule: "pattern.chance".into(),
                field: "chance".into(),
                from: bounded.chance,
                to: chance,
            });
            bounded.chance = chance;
        }
        (bounded, decisions)
    }
}

/// `list` entries may be exact ids or `prefix.*` globs; empty list = deny all.
fn list_matches(list: &[String], value: &str) -> bool {
    list.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix(".*") {
            value == prefix || value.starts_with(&format!("{prefix}."))
        } else if entry == "*" {
            true
        } else {
            entry == value
        }
    })
}

fn merged_channel_limits(policy: &PolicyConfig, channel: &str) -> interaction_core::ChannelLimits {
    let wildcard = policy.channel_limits.get("*");
    let specific = policy.channel_limits.get(channel);
    let mut merged = interaction_core::ChannelLimits::default();
    for source in [wildcard, specific].into_iter().flatten() {
        if source.max_magnitude.is_some() {
            merged.max_magnitude = min_opt_f64(merged.max_magnitude, source.max_magnitude);
        }
        if source.max_duration_ms.is_some() {
            merged.max_duration_ms = min_opt_u64(merged.max_duration_ms, source.max_duration_ms);
        }
        if source.max_per_hour.is_some() {
            merged.max_per_hour = min_opt_u32(merged.max_per_hour, source.max_per_hour);
        }
        if source.cooldown_ms.is_some() {
            // For cooldown the stricter value is the larger one.
            merged.cooldown_ms = max_opt_u64(merged.cooldown_ms, source.cooldown_ms);
        }
        if source.session_budget_ms.is_some() {
            merged.session_budget_ms =
                min_opt_u64(merged.session_budget_ms, source.session_budget_ms);
        }
    }
    merged
}

fn min_opt_f64(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

fn min_opt_u64(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

fn max_opt_u64(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

fn min_opt_u32(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Returns the active quiet window at `local_time`, if any. Windows may wrap
/// midnight (e.g. 22:00 - 08:00).
fn active_quiet_window(windows: &[QuietHours], local_time: NaiveTime) -> Option<&QuietHours> {
    windows.iter().find(|w| {
        let (Some(start), Some(end)) = (parse_hhmm(&w.start), parse_hhmm(&w.end)) else {
            return false;
        };
        if start <= end {
            local_time >= start && local_time < end
        } else {
            local_time >= start || local_time < end
        }
    })
}

fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_core::*;

    fn manifest(id: &str, channel: &str, risk: RiskClass) -> ActuatorManifest {
        ActuatorManifest {
            id: ActuatorId::new(id),
            name: id.into(),
            description: "test".into(),
            channel: channel.into(),
            capabilities: vec![],
            parameters_schema: None,
            supports_cancel: true,
            supports_pattern: false,
            requires_consent: false,
            external_side_effect: false,
            reversible: true,
            risk_class: risk,
            latency_ms: None,
            cost: CostDescriptor::default(),
            limits: ActuatorLimits {
                max_magnitude: Some(0.8),
                max_duration_ms: Some(10_000),
                ..Default::default()
            },
            health: ComponentHealth::healthy(),
            availability: Availability::Available,
            driver: "test".into(),
            version: "0".into(),
            schema_version: SCHEMA_VERSION.into(),
            human: None,
        }
    }

    fn base_req<'a>(
        actuator: &'a ActuatorManifest,
        requested: &'a ActionParameters,
    ) -> AuthorizationRequest<'a> {
        AuthorizationRequest {
            actuator,
            requested,
            intent: "test",
            source: ActionSource::ExplicitRequest,
            local_time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            now: chrono::Utc::now(),
            emergency_stop_engaged: false,
        }
    }

    fn policy_with(actuator: &str, channel: &str) -> PolicyConfig {
        let mut p = PolicyConfig::default();
        p.actuator_allowlist.push(actuator.to_string());
        p.allowed_channels.push(channel.to_string());
        p
    }

    #[test]
    fn effective_limit_is_min_of_all_caps() {
        let mut m = manifest("haptic.mock", "haptic", RiskClass::BoundedSideEffect);
        m.limits.max_magnitude = Some(0.6); // device safe limit
        let mut policy = policy_with("haptic.mock", "haptic");
        policy.channel_limits.insert(
            "haptic".into(),
            ChannelLimits {
                max_magnitude: Some(0.5),
                ..Default::default()
            }, // user preference
        );
        let session = Session::new(chrono::Utc::now(), None, None);
        let requested = ActionParameters {
            magnitude: Some(0.9),
            ..Default::default()
        };
        let req = base_req(&m, &requested);
        let result = Governor::authorize(&policy, &session, &req, &UsageContext::default());
        assert_eq!(result.outcome, AuthorizationOutcome::Authorized);
        assert_eq!(result.effective.magnitude, Some(0.5)); // min(0.9, 0.5, 0.6)
        assert!(result
            .decisions
            .iter()
            .any(|d| matches!(d, PolicyDecision::Clamped { .. })));
    }

    #[test]
    fn emergency_stop_blocks_everything() {
        let m = manifest("conversation", "conversation", RiskClass::Low);
        let policy = policy_with("conversation", "conversation");
        let session = Session::new(chrono::Utc::now(), None, None);
        let requested = ActionParameters::default();
        let mut req = base_req(&m, &requested);
        req.emergency_stop_engaged = true;
        let r = Governor::authorize(&policy, &session, &req, &UsageContext::default());
        assert_eq!(r.outcome, AuthorizationOutcome::Blocked);
    }

    #[test]
    fn unlisted_actuator_is_blocked() {
        let m = manifest("rogue.device", "haptic", RiskClass::High);
        let policy = PolicyConfig::default();
        let session = Session::new(chrono::Utc::now(), None, None);
        let requested = ActionParameters::default();
        let req = base_req(&m, &requested);
        let r = Governor::authorize(&policy, &session, &req, &UsageContext::default());
        assert_eq!(r.outcome, AuthorizationOutcome::Blocked);
    }

    #[test]
    fn high_risk_requires_approval_then_passes_with_consent() {
        let m = manifest("device.serial", "haptic", RiskClass::High);
        let mut policy = policy_with("device.serial", "haptic");
        policy.initiative = InitiativeLevel::Active;
        let now = chrono::Utc::now();
        let mut session = Session::new(now, None, None);
        let requested = ActionParameters {
            magnitude: Some(0.3),
            ..Default::default()
        };
        let req = base_req(&m, &requested);
        let r = Governor::authorize(&policy, &session, &req, &UsageContext::default());
        assert_eq!(r.outcome, AuthorizationOutcome::ApprovalRequired);

        session.grant(ConsentScope::Actuator("device.serial".into()), now, None);
        let r2 = Governor::authorize(&policy, &session, &req, &UsageContext::default());
        assert_eq!(r2.outcome, AuthorizationOutcome::Authorized);
    }

    #[test]
    fn quiet_hours_block_audio_but_not_conversation() {
        let audio = manifest("audio.player", "audio", RiskClass::BoundedSideEffect);
        let convo = manifest("conversation", "conversation", RiskClass::Low);
        let mut policy = PolicyConfig::default();
        policy.actuator_allowlist.push("audio.player".into());
        policy.allowed_channels.push("audio".into());
        policy.quiet_hours.push(QuietHours {
            start: "22:00".into(),
            end: "08:00".into(),
            silenced_channels: vec![],
        });
        let session = Session::new(chrono::Utc::now(), None, None);
        let requested = ActionParameters::default();
        let night = NaiveTime::from_hms_opt(23, 30, 0).unwrap();

        let mut req = base_req(&audio, &requested);
        req.local_time = night;
        let r = Governor::authorize(&policy, &session, &req, &UsageContext::default());
        assert_eq!(r.outcome, AuthorizationOutcome::Blocked);

        let mut req2 = base_req(&convo, &requested);
        req2.local_time = night;
        let r2 = Governor::authorize(&policy, &session, &req2, &UsageContext::default());
        assert_eq!(r2.outcome, AuthorizationOutcome::Authorized);
    }

    #[test]
    fn cooldown_and_hourly_limits() {
        let m = manifest("notify", "notification", RiskClass::BoundedSideEffect);
        let mut policy = policy_with("notify", "notification");
        policy.channel_limits.insert(
            "notification".into(),
            ChannelLimits {
                cooldown_ms: Some(60_000),
                max_per_hour: Some(3),
                ..Default::default()
            },
        );
        let now = chrono::Utc::now();
        let session = Session::new(now, None, None);
        let requested = ActionParameters::default();
        let req = base_req(&m, &requested);

        let cooling = UsageContext {
            actuator_last_fired_at: Some(now - chrono::Duration::seconds(10)),
            ..Default::default()
        };
        assert_eq!(
            Governor::authorize(&policy, &session, &req, &cooling).outcome,
            AuthorizationOutcome::Blocked
        );

        let saturated = UsageContext {
            actuator_fired_last_hour: 3,
            ..Default::default()
        };
        assert_eq!(
            Governor::authorize(&policy, &session, &req, &saturated).outcome,
            AuthorizationOutcome::Blocked
        );

        let fresh = UsageContext {
            actuator_last_fired_at: Some(now - chrono::Duration::seconds(120)),
            actuator_fired_last_hour: 2,
            ..Default::default()
        };
        assert_eq!(
            Governor::authorize(&policy, &session, &req, &fresh).outcome,
            AuthorizationOutcome::Authorized
        );
    }

    #[test]
    fn session_budget_clamps_then_blocks() {
        let m = manifest("haptic.mock", "haptic", RiskClass::BoundedSideEffect);
        let mut policy = policy_with("haptic.mock", "haptic");
        policy.channel_limits.insert(
            "haptic".into(),
            ChannelLimits {
                session_budget_ms: Some(5_000),
                ..Default::default()
            },
        );
        let session = Session::new(chrono::Utc::now(), None, None);
        let requested = ActionParameters {
            duration_ms: Some(4_000),
            ..Default::default()
        };
        let req = base_req(&m, &requested);

        let partly_used = UsageContext {
            channel_budget_used_ms: 3_000,
            ..Default::default()
        };
        let r = Governor::authorize(&policy, &session, &req, &partly_used);
        assert_eq!(r.outcome, AuthorizationOutcome::Authorized);
        assert_eq!(r.effective.duration_ms, Some(2_000)); // clamped to remaining

        let exhausted = UsageContext {
            channel_budget_used_ms: 5_000,
            ..Default::default()
        };
        assert_eq!(
            Governor::authorize(&policy, &session, &req, &exhausted).outcome,
            AuthorizationOutcome::Blocked
        );
    }

    #[test]
    fn autonomous_respects_initiative() {
        let m = manifest("web-ui", "web-ui", RiskClass::Low);
        let mut policy = policy_with("web-ui", "web-ui");
        policy.initiative = InitiativeLevel::Passive;
        let session = Session::new(chrono::Utc::now(), None, None);
        let requested = ActionParameters::default();
        let mut req = base_req(&m, &requested);
        req.source = ActionSource::Autonomous;
        assert_eq!(
            Governor::authorize(&policy, &session, &req, &UsageContext::default()).outcome,
            AuthorizationOutcome::Blocked
        );
    }

    #[test]
    fn pattern_bounding_clamps_steps_and_magnitude() {
        let m = manifest("haptic.mock", "haptic", RiskClass::BoundedSideEffect);
        let policy = PolicyConfig {
            max_pattern_steps: 4,
            ..PolicyConfig::default()
        };
        let pattern = PatternSpec {
            steps: (0..10)
                .map(|_| PatternStep {
                    magnitude: 1.0,
                    duration_ms: 100,
                    pause_ms: 50,
                })
                .collect(),
            repeat: 2,
            chance: 1.5,
            jitter_ms: 10,
        };
        let (bounded, decisions) = Governor::bound_pattern(&policy, &m, &pattern, 0.5);
        assert_eq!(bounded.steps.len(), 4);
        assert!(bounded.steps.iter().all(|s| s.magnitude <= 0.5));
        assert!((bounded.chance - 1.0).abs() < f64::EPSILON);
        assert!(decisions.len() >= 3);
    }
}
