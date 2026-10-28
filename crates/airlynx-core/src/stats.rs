//! Process-wide counters for judging what an idle receiver costs: how often its background
//! loops wake up, and how much CPU time the process has used.

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static ACCEPT_POLLS: AtomicU64 = AtomicU64::new(0);
static DISCOVERY_POLLS: AtomicU64 = AtomicU64::new(0);
static DISCOVERY_REPLIES: AtomicU64 = AtomicU64::new(0);
static SESSIONS: AtomicU64 = AtomicU64::new(0);

/// Linux reports CPU time in clock ticks, and `USER_HZ` is 100 on Linux and Android.
const CLOCK_TICKS_PER_SECOND: f64 = 100.0;

pub(crate) fn accept_poll() {
    ACCEPT_POLLS.fetch_add(1, Relaxed);
}

pub(crate) fn discovery_poll() {
    DISCOVERY_POLLS.fetch_add(1, Relaxed);
}

pub(crate) fn discovery_reply() {
    DISCOVERY_REPLIES.fetch_add(1, Relaxed);
}

pub(crate) fn session() {
    SESSIONS.fetch_add(1, Relaxed);
}

/// Counters since the process started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Wakeups of the TCP accept loop.
    pub accept_polls: u64,
    /// Wakeups of the discovery responder loop (a packet or a read timeout).
    pub discovery_polls: u64,
    /// Discovery requests that were answered.
    pub discovery_replies: u64,
    /// Incoming transfer connections.
    pub sessions: u64,
}

pub fn snapshot() -> Stats {
    Stats {
        accept_polls: ACCEPT_POLLS.load(Relaxed),
        discovery_polls: DISCOVERY_POLLS.load(Relaxed),
        discovery_replies: DISCOVERY_REPLIES.load(Relaxed),
        sessions: SESSIONS.load(Relaxed),
    }
}

/// CPU time (user plus system) this process has used so far. `None` where `/proc` does not exist.
pub fn process_cpu_seconds() -> Option<f64> {
    let ticks = parse_cpu_ticks(&fs::read_to_string("/proc/self/stat").ok()?)?;
    Some(ticks as f64 / CLOCK_TICKS_PER_SECOND)
}

/// Number of threads in this process. `None` where `/proc` does not exist.
pub fn thread_count() -> Option<u32> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| line.strip_prefix("Threads:")?.trim().parse().ok())
}

/// `utime + stime` from the contents of `/proc/<pid>/stat`.
fn parse_cpu_ticks(stat: &str) -> Option<u64> {
    // The command name is in parentheses and may contain spaces, so count fields after the last ')'.
    // Those start at field 3 (the state), which makes utime field 14 and stime field 15.
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace().skip(11);
    let utime: u64 = fields.next()?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some(utime + stime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_ticks_skip_the_command_name() {
        let stat = "4242 (air lynx) (x) S 1 4242 4242 0 -1 4194560 100 0 0 0 150 25 0 0 20 0 9 0 1000 0 0";
        assert_eq!(parse_cpu_ticks(stat), Some(175));
    }

    #[test]
    fn cpu_ticks_reject_truncated_input() {
        assert_eq!(parse_cpu_ticks("1 (a) S 1 2"), None);
        assert_eq!(parse_cpu_ticks("no parentheses"), None);
    }

    #[test]
    fn counters_only_go_up() {
        let before = snapshot();
        accept_poll();
        session();
        let after = snapshot();
        assert!(after.accept_polls > before.accept_polls);
        assert!(after.sessions > before.sessions);
    }
}
