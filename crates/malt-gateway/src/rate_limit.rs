//! Monotonic, per-caller fixed windows. Expired state is reclaimed on access.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
#[error("rate-limit allowance and window must be positive")]
pub struct InvalidRateLimit;

#[derive(Debug)]
struct Allowance {
    used: usize,
    window_started: Instant,
}

#[derive(Debug)]
struct State {
    callers: HashMap<String, Allowance>,
    last_cleanup: Instant,
}

#[derive(Debug)]
pub struct RateLimiter {
    max_per_window: usize,
    window: Duration,
    state: Mutex<State>,
}

impl RateLimiter {
    /// Preserve the caller-selected allowance with a 60-second window.
    /// A zero allowance denies all requests.
    pub fn new(max_per_window: usize) -> Self {
        Self::create(max_per_window, DEFAULT_WINDOW)
    }

    /// Configure a positive allowance and monotonic window.
    pub fn with_window(max_per_window: usize, window: Duration) -> Result<Self, InvalidRateLimit> {
        if max_per_window == 0 || window.is_zero() {
            return Err(InvalidRateLimit);
        }
        Ok(Self::create(max_per_window, window))
    }

    fn create(max_per_window: usize, window: Duration) -> Self {
        Self {
            max_per_window,
            window,
            state: Mutex::new(State {
                callers: HashMap::new(),
                last_cleanup: Instant::now(),
            }),
        }
    }

    /// Consume allowance. Time alone restores an expired window; neither a
    /// timer thread nor an externally invoked reset is needed.
    pub fn check(&self, client_id: &str) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if now.duration_since(state.last_cleanup) >= self.window {
            state
                .callers
                .retain(|_, allowance| now.duration_since(allowance.window_started) < self.window);
            // Removing keys alone retains the peak hash allocation forever.
            if state.callers.capacity() > state.callers.len().saturating_mul(4) {
                state.callers.shrink_to_fit();
            }
            state.last_cleanup = now;
        }
        let allowance = state
            .callers
            .entry(client_id.to_owned())
            .or_insert(Allowance {
                used: 0,
                window_started: now,
            });
        if now.duration_since(allowance.window_started) >= self.window {
            allowance.used = 0;
            allowance.window_started = now;
        }
        if allowance.used >= self.max_per_window {
            return false;
        }
        allowance.used += 1;
        true
    }
}
