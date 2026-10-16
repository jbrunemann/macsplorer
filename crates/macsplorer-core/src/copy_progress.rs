//! When the Copying... / Moving... progress dialog shows and closes, and
//! what it says about the time left.
//!
//! The timing is shell32's `CProgressDialog` (build 10240), which the copy
//! engine reports to: `GetDialogDisplayDelayTime` is 1000 ms, and
//! `CProgressDialogUI::_HandleTimerMessage` shows the dialog once the
//! operation has made progress for that long (timer 1, measured with
//! `CProgressTimer::GetMillisecondsProgress`, which `PauseTimer` stops while
//! a question is up). When the operation ends, a dialog that was shown stays
//! until it has been up for 1000 ms, so it never just flashes (timer 2); one
//! that never showed ends at once.
//!
//! Not traced: the estimate (the average rate so far, refreshed once a
//! second) and its wording, which is Windows 2000's from memory; the
//! strings are in shell32's MUI file, which isn't analyzed.

/// Seconds of progress before the dialog shows.
pub const SHOW_DELAY: f64 = 1.0;
/// Seconds a shown dialog stays up at least.
pub const MIN_SHOWN: f64 = 1.0;
/// Seconds between updates of the time left.
pub const ESTIMATE_EVERY: f64 = 1.0;

/// Whether the dialog is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not yet, or never: a short operation shows nothing.
    Hidden,
    Shown,
    /// Gone for good.
    Closed,
}

/// The two timers of the dialog, and its time-left line.
#[derive(Debug, Default)]
pub struct DialogClock {
    /// Seconds of progress, which don't run while a question is up.
    progress: f64,
    /// When the last frame was seen, on the real clock.
    last: Option<f64>,
    /// When the dialog showed, on the real clock.
    shown_at: Option<f64>,
    estimate: Option<(f64, String)>,
}

impl DialogClock {
    /// Moves the clocks to `now` (seconds, on any clock that only goes
    /// forward). `paused` while the operation waits for an answer.
    pub fn tick(&mut self, now: f64, paused: bool, finished: bool) -> Phase {
        let step = self.last.map_or(0.0, |last| (now - last).max(0.0));
        self.last = Some(now);
        if !paused && !finished {
            self.progress += step;
        }
        match self.shown_at {
            None if finished => Phase::Closed,
            None if self.progress >= SHOW_DELAY => {
                self.shown_at = Some(now);
                Phase::Shown
            }
            None => Phase::Hidden,
            Some(at) if finished && now - at >= MIN_SHOWN => Phase::Closed,
            Some(_) => Phase::Shown,
        }
    }

    /// Seconds until the dialog may show or close, for scheduling a repaint.
    pub fn wait(&self, now: f64, finished: bool) -> Option<f64> {
        match self.shown_at {
            None if finished => None,
            None => Some((SHOW_DELAY - self.progress).max(0.0)),
            Some(at) if finished => Some((at + MIN_SHOWN - now).max(0.0)),
            Some(_) => None,
        }
    }

    /// The time-left line for `done` of `total` units, kept for
    /// [`ESTIMATE_EVERY`] seconds of progress so it doesn't flicker. Empty
    /// until anything is done.
    pub fn time_left(&mut self, done: u64, total: u64) -> &str {
        let fresh = self.estimate.as_ref().is_some_and(|(at, _)| self.progress - at < ESTIMATE_EVERY);
        if !fresh {
            let text = remaining(self.progress, done, total).map(remaining_text).unwrap_or_default();
            self.estimate = Some((self.progress, text));
        }
        self.estimate.as_ref().map_or("", |(_, t)| t)
    }
}

/// Seconds left at the average rate so far.
pub fn remaining(elapsed: f64, done: u64, total: u64) -> Option<f64> {
    if done == 0 || total == 0 || elapsed <= 0.0 {
        return None;
    }
    Some(elapsed * total.saturating_sub(done) as f64 / done as f64)
}

/// "12 Seconds Remaining", "3 Minutes Remaining": counted up, so the last
/// second still says one.
pub fn remaining_text(secs: f64) -> String {
    let secs = secs.ceil().max(1.0) as u64;
    match secs {
        1 => "1 Second Remaining".into(),
        2..60 => format!("{secs} Seconds Remaining"),
        60..120 => "1 Minute Remaining".into(),
        _ => format!("{} Minutes Remaining", secs.div_ceil(60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_operations_show_nothing() {
        let mut c = DialogClock::default();
        assert_eq!(c.tick(0.0, false, false), Phase::Hidden);
        assert_eq!(c.tick(0.9, false, false), Phase::Hidden);
        assert!((c.wait(0.9, false).unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(c.tick(0.95, false, true), Phase::Closed);
    }

    #[test]
    fn shows_after_a_second_of_progress_and_stays_a_second() {
        let mut c = DialogClock::default();
        c.tick(0.0, false, false);
        assert_eq!(c.tick(1.0, false, false), Phase::Shown);
        assert_eq!(c.tick(1.2, false, true), Phase::Shown);
        assert!((c.wait(1.2, true).unwrap() - 0.8).abs() < 1e-9);
        assert_eq!(c.tick(2.0, false, true), Phase::Closed);
    }

    #[test]
    fn a_question_stops_the_clock() {
        let mut c = DialogClock::default();
        c.tick(0.0, false, false);
        c.tick(0.5, false, false);
        assert_eq!(c.tick(5.0, true, false), Phase::Hidden);
        assert_eq!(c.tick(5.4, false, false), Phase::Hidden);
        assert_eq!(c.tick(5.5, false, false), Phase::Shown);
    }

    #[test]
    fn estimates() {
        assert_eq!(remaining(2.0, 0, 100), None);
        assert_eq!(remaining(2.0, 25, 100), Some(6.0));
        assert_eq!(remaining_text(0.2), "1 Second Remaining");
        assert_eq!(remaining_text(11.1), "12 Seconds Remaining");
        assert_eq!(remaining_text(60.0), "1 Minute Remaining");
        assert_eq!(remaining_text(121.0), "3 Minutes Remaining");
    }

    #[test]
    fn the_time_left_changes_once_a_second() {
        let mut c = DialogClock::default();
        c.tick(0.0, false, false);
        c.tick(2.0, false, false);
        assert_eq!(c.time_left(25, 100), "6 Seconds Remaining");
        c.tick(2.5, false, false);
        assert_eq!(c.time_left(90, 100), "6 Seconds Remaining");
        c.tick(3.0, false, false);
        assert_eq!(c.time_left(90, 100), "1 Second Remaining");
    }
}
