//! How much one address may ask of the sign-in: password checks a minute,
//! and new accounts a day. Kept in memory: a restart forgets them.
//!
//! An IPv6 address counts by its /64 (one home network), an IPv4 one whole.
//! This machine (loopback) is never limited, so local tests and an admin on
//! the server itself are not.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::LockExt;

/// Password checks one address may ask for in [`ATTEMPT_WINDOW`]:
/// `SRP_SIGN_INS_PER_MINUTE`.
pub const DEFAULT_ATTEMPTS: usize = 20;
pub const ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
/// New accounts one address may register in [`REGISTRATION_WINDOW`]:
/// `SRP_REGISTRATIONS_PER_DAY` (0: no limit).
pub const DEFAULT_REGISTRATIONS: usize = 3;
pub const REGISTRATION_WINDOW: Duration = Duration::from_secs(24 * 3600);
/// Addresses remembered at most; past it, those with nothing recent go.
const MAX_ADDRESSES: usize = 100_000;

pub struct Limits {
    attempts: Window,
    registrations: Window,
}

impl Limits {
    pub fn new(attempts: usize, registrations: usize) -> Self {
        Self {
            attempts: Window::new(attempts, ATTEMPT_WINDOW),
            registrations: Window::new(registrations, REGISTRATION_WINDOW),
        }
    }

    /// The limits srp's environment asks for.
    pub fn from_env() -> Self {
        let read = |name: &str, default: usize| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(default)
        };
        Self::new(
            read("SRP_SIGN_INS_PER_MINUTE", DEFAULT_ATTEMPTS),
            read("SRP_REGISTRATIONS_PER_DAY", DEFAULT_REGISTRATIONS),
        )
    }

    /// One more password check from `ip`; `false` if it has had its fill.
    pub fn attempt(&self, ip: IpAddr, now: Instant) -> bool {
        self.attempts.take(ip, now)
    }

    /// Whether `ip` may register another account now (without counting it).
    pub fn may_register(&self, ip: IpAddr, now: Instant) -> bool {
        self.registrations.has_room(ip, now)
    }

    /// `ip` registered an account.
    pub fn registered(&self, ip: IpAddr, now: Instant) {
        self.registrations.take(ip, now);
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new(DEFAULT_ATTEMPTS, DEFAULT_REGISTRATIONS)
    }
}

/// At most `most` events per address in any `span`; 0: no limit.
struct Window {
    most: usize,
    span: Duration,
    events: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
}

impl Window {
    fn new(most: usize, span: Duration) -> Self {
        Self {
            most,
            span,
            events: Mutex::new(HashMap::new()),
        }
    }

    fn has_room(&self, ip: IpAddr, now: Instant) -> bool {
        if self.most == 0 || ip.to_canonical().is_loopback() {
            return true;
        }
        let mut events = self.events.locked();
        let Some(times) = events.get_mut(&bucket(ip)) else {
            return true;
        };
        times.retain(|at| now.saturating_duration_since(*at) < self.span);
        times.len() < self.most
    }

    fn take(&self, ip: IpAddr, now: Instant) -> bool {
        if self.most == 0 || ip.to_canonical().is_loopback() {
            return true;
        }
        let mut events = self.events.locked();
        if events.len() >= MAX_ADDRESSES {
            events.retain(|_, times| {
                times
                    .back()
                    .is_some_and(|at| now.saturating_duration_since(*at) < self.span)
            });
        }
        let times = events.entry(bucket(ip)).or_default();
        times.retain(|at| now.saturating_duration_since(*at) < self.span);
        if times.len() >= self.most {
            return false;
        }
        times.push_back(now);
        true
    }
}

/// What an address counts as: itself, or its /64 for IPv6.
fn bucket(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let mut segments = v6.segments();
            segments[4..].fill(0);
            IpAddr::V6(segments.into())
        }
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn an_address_gets_its_fill_then_waits_for_the_window() {
        let limits = Limits::new(3, 2);
        let now = Instant::now();
        for _ in 0..3 {
            assert!(limits.attempt(ip("203.0.113.1"), now));
        }
        assert!(!limits.attempt(ip("203.0.113.1"), now));
        // Another address, and this machine, are not held back.
        assert!(limits.attempt(ip("203.0.113.2"), now));
        for _ in 0..10 {
            assert!(limits.attempt(ip("127.0.0.1"), now));
        }
        assert!(limits.attempt(ip("203.0.113.1"), now + ATTEMPT_WINDOW));
    }

    #[test]
    fn registrations_count_per_day_and_ipv6_by_its_network() {
        let limits = Limits::new(100, 2);
        let now = Instant::now();
        limits.registered(ip("2001:db8:1:2::1"), now);
        limits.registered(ip("2001:db8:1:2::ffff"), now);
        assert!(!limits.may_register(ip("2001:db8:1:2:abcd::9"), now));
        assert!(limits.may_register(ip("2001:db8:1:3::1"), now));
        assert!(limits.may_register(ip("2001:db8:1:2::1"), now + REGISTRATION_WINDOW));
        // 0: no limit.
        let open = Limits::new(0, 0);
        for _ in 0..100 {
            assert!(open.attempt(ip("198.51.100.1"), now));
        }
    }
}
