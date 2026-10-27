//! Monotonic fixed windows, shared admission, and access-driven reclamation.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
#[error("rate-limit allowances and window must be positive")]
pub struct InvalidRateLimit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LimitCause {
    PerCaller,
    SystemWide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateSnapshot {
    pub limit: usize,
    pub remaining: usize,
    pub reset: Duration,
    pub global_limit: usize,
    pub global_remaining: usize,
    pub global_reset: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateRefusal {
    pub retry_after: Duration,
    pub cause: LimitCause,
    pub snapshot: RateSnapshot,
}

/// HTTP delta-seconds are integral: round up so retry never precedes expiry.
pub fn delta_seconds(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() != 0))
}

#[derive(Debug)]
struct Allowance {
    used: usize,
    window_started: Instant,
}

impl Allowance {
    fn reset_if_elapsed(&mut self, now: Instant, window: Duration) {
        if now.duration_since(self.window_started) >= window {
            self.used = 0;
            self.window_started = now;
        }
    }

    fn remaining_time(&self, now: Instant, window: Duration) -> Duration {
        window.saturating_sub(now.duration_since(self.window_started))
    }
}

#[derive(Debug)]
struct State {
    callers: HashMap<String, Allowance>,
    global: Allowance,
    last_cleanup: Instant,
}

#[derive(Debug)]
pub struct RateLimiter {
    max_per_window: usize,
    global_max: usize,
    window: Duration,
    state: Mutex<State>,
}

impl RateLimiter {
    /// A zero caller allowance denies all requests. Production configuration
    /// rejects zero and uses the validated configured constructor.
    pub fn new(max_per_window: usize) -> Self {
        Self::create(
            max_per_window,
            max_per_window.saturating_mul(10).max(1000),
            DEFAULT_WINDOW,
        )
    }

    pub fn with_window(max_per_window: usize, window: Duration) -> Result<Self, InvalidRateLimit> {
        Self::with_limits(
            max_per_window,
            max_per_window.saturating_mul(10).max(1000),
            window,
        )
    }

    pub fn with_limits(
        max_per_window: usize,
        global_max: usize,
        window: Duration,
    ) -> Result<Self, InvalidRateLimit> {
        if max_per_window == 0 || global_max == 0 || window.is_zero() {
            return Err(InvalidRateLimit);
        }
        Ok(Self::create(max_per_window, global_max, window))
    }

    fn create(max_per_window: usize, global_max: usize, window: Duration) -> Self {
        let now = Instant::now();
        Self {
            max_per_window,
            global_max,
            window,
            state: Mutex::new(State {
                callers: HashMap::new(),
                global: Allowance {
                    used: 0,
                    window_started: now,
                },
                last_cleanup: now,
            }),
        }
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    /// Compatibility convenience for callers that only need admission.
    pub fn check(&self, client_id: &str) -> bool {
        self.admit(client_id).is_ok()
    }

    fn maintain(&self, state: &mut State, now: Instant) {
        state.global.reset_if_elapsed(now, self.window);
        if now.duration_since(state.last_cleanup) >= self.window {
            state
                .callers
                .retain(|_, value| value.remaining_time(now, self.window) > Duration::ZERO);
            if state.callers.capacity() > state.callers.len().saturating_mul(4) {
                state.callers.shrink_to_fit();
            }
            state.last_cleanup = now;
        }
    }

    fn snapshot(&self, state: &State, client_id: Option<&str>, now: Instant) -> RateSnapshot {
        let (remaining, reset) = match client_id {
            Some(id) => match state.callers.get(id) {
                Some(value) if !value.remaining_time(now, self.window).is_zero() => (
                    self.max_per_window.saturating_sub(value.used),
                    value.remaining_time(now, self.window),
                ),
                _ => (self.max_per_window, self.window),
            },
            // No authenticated identity has an allowance to spend.
            None => (0, Duration::ZERO),
        };
        RateSnapshot {
            limit: self.max_per_window,
            remaining,
            reset,
            global_limit: self.global_max,
            global_remaining: self.global_max.saturating_sub(state.global.used),
            global_reset: state.global.remaining_time(now, self.window),
        }
    }

    /// Observe without consuming quota, including for unauthenticated replies.
    pub fn observe(&self, client_id: Option<&str>) -> RateSnapshot {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        self.maintain(&mut state, now);
        self.snapshot(&state, client_id, now)
    }

    /// Check both budgets and charge them under one lock. Rate-refused
    /// requests charge neither; scope-refused requests are already admitted.
    pub fn admit(&self, client_id: &str) -> Result<RateSnapshot, RateRefusal> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        self.maintain(&mut state, now);
        let snapshot = self.snapshot(&state, Some(client_id), now);
        if snapshot.global_remaining == 0 {
            return Err(RateRefusal {
                cause: LimitCause::SystemWide,
                retry_after: if snapshot.remaining == 0 {
                    snapshot.global_reset.max(snapshot.reset)
                } else {
                    snapshot.global_reset
                },
                snapshot,
            });
        }
        if snapshot.remaining == 0 {
            return Err(RateRefusal {
                cause: LimitCause::PerCaller,
                retry_after: snapshot.reset,
                snapshot,
            });
        }
        let allowance = state
            .callers
            .entry(client_id.to_owned())
            .or_insert(Allowance {
                used: 0,
                window_started: now,
            });
        allowance.reset_if_elapsed(now, self.window);
        allowance.used += 1;
        state.global.used += 1;
        Ok(self.snapshot(&state, Some(client_id), now))
    }
}
