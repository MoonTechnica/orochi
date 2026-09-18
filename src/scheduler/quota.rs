use crate::{
    agents::{AgentError, ErrorKind, QuotaObservation},
    types::*,
};

pub fn fail(state: &mut RuntimeState, error: &AgentError, time: i64) {
    state.last_failure_at = Some(time);
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    if error.kind == ErrorKind::RateLimit {
        state.rate_limit_count = state.rate_limit_count.saturating_add(1);
    }
    if error.kind == ErrorKind::RateLimit
        || error.kind == ErrorKind::Authentication
        || error.kind == ErrorKind::Unavailable
        || state.consecutive_failures >= 3
    {
        let backoff =
            [300, 900, 1800, 3600][state.consecutive_failures.saturating_sub(1).min(3) as usize];
        state.cooldown_until = Some(error.reset_at.unwrap_or(time.saturating_add(backoff)));
        state.status = RuntimeStatus::Cooldown;
    }
}
pub fn succeed(state: &mut RuntimeState, time: i64) {
    state.last_success_at = Some(time);
    state.consecutive_failures = 0;
    state.cooldown_until = None;
    if state.reset_at.is_some_and(|t| t <= time) {
        state.quota_estimate = None;
        state.reset_at = None;
    }
    state.status = if state.quota_estimate.is_some_and(|q| q < 0.15) {
        RuntimeStatus::SoftLimit
    } else {
        RuntimeStatus::Available
    };
}
pub fn observe(state: &mut RuntimeState, quota: &QuotaObservation, time: i64) {
    state.quota_estimate = Some(quota.remaining);
    state.reset_at = quota.reset_at;
    if quota.remaining <= 0.0 {
        state.status = RuntimeStatus::Cooldown;
        state.cooldown_until = Some(quota.reset_at.filter(|t| *t > time).unwrap_or(time + 300));
    } else if state.available(time) {
        state.status = if quota.remaining < 0.15 {
            RuntimeStatus::SoftLimit
        } else {
            RuntimeStatus::Available
        };
    }
}
pub fn available(map: &RuntimeMap, agent: &str, model: &str, time: i64) -> bool {
    ["*", model].iter().all(|m| {
        map.get(&(agent.into(), (*m).into()))
            .is_none_or(|s| s.available(time))
    })
}
pub fn shadow_price(map: &RuntimeMap, agent: &str, model: &str, time: i64) -> f64 {
    ["*", model]
        .iter()
        .filter_map(|m| map.get(&(agent.into(), (*m).into())))
        .map(|s| s.shadow_price(time))
        .fold(1.0, f64::max)
}
