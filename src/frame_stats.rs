//! How often the window of the program is drawn, and why.
//!
//! The window is drawn only when something asks for it (a click, a change, a timer), so a
//! program that has nothing to show should hardly draw at all.  When it draws a great many times
//! a second anyway, the thread that draws is busy all the time, which is the most that one thread
//! can cost.  This counts the draws, says (in the log and in Power Tools) what asked for them the
//! most, and keeps a window that is not in use (not in front) from being drawn more than a few
//! times a second whatever asks.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long the draws are counted before they are told of.
const WINDOW: Duration = Duration::from_secs(5);
/// More draws than this a second are told of.
const MANY_PER_SECOND: f32 = 20.;
/// A window that is not in front is not drawn more often than this when it is drawn a great deal.
const UNUSED_MIN_GAP: Duration = Duration::from_millis(50);
/// What counts as a great deal (draws a second, over the last window).
const A_GREAT_DEAL: f32 = 25.;

pub struct FrameStats {
    since: Instant,
    frames: u32,
    busy: Duration,
    causes: HashMap<String, u32>,
    started: Option<Instant>,
    last_end: Option<Instant>,
    /// Draws a second over the last window.
    pub per_second: f32,
    /// Part of the time spent drawing over the last window, 0 to 1.
    pub busy_share: f32,
    /// What asked for the draws the most over the last window.
    pub top_cause: String,
    /// Draws were held back, because the window is not in use and was drawn a great deal.
    pub throttled: bool,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            since: Instant::now(),
            frames: 0,
            busy: Duration::ZERO,
            causes: HashMap::new(),
            started: None,
            last_end: None,
            per_second: 0.,
            busy_share: 0.,
            top_cause: String::new(),
            throttled: false,
        }
    }
}

impl FrameStats {
    /// Call at the start of a draw.  `focused`: the window is the one in use.
    pub fn begin(&mut self, ctx: &egui::Context, focused: bool) {
        // A window that is not in use, drawn a great deal (by something that asks for it all
        // the time): it waits, so that it costs a little.
        self.throttled = !focused && self.per_second > A_GREAT_DEAL;
        if self.throttled
            && let Some(end) = self.last_end
        {
            let since = end.elapsed();
            if since < UNUSED_MIN_GAP {
                std::thread::sleep(UNUSED_MIN_GAP - since);
            }
        }
        for cause in ctx.repaint_causes() {
            *self.causes.entry(format!("{}:{} {}", cause.file, cause.line, cause.reason)).or_default() += 1;
        }
        self.started = Some(Instant::now());
    }

    /// Call at the end of a draw.
    pub fn end(&mut self) {
        let now = Instant::now();
        if let Some(started) = self.started.take() {
            self.busy += now.duration_since(started);
        }
        self.last_end = Some(now);
        self.frames += 1;
        let elapsed = now.duration_since(self.since);
        if elapsed >= WINDOW {
            let seconds = elapsed.as_secs_f32();
            self.per_second = self.frames as f32 / seconds;
            self.busy_share = (self.busy.as_secs_f32() / seconds).min(1.);
            let mut ranked: Vec<(&String, &u32)> = self.causes.iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(a.1));
            self.top_cause = ranked.first().map(|(cause, count)| format!("{cause} ({count} of {})", self.frames)).unwrap_or_default();
            if self.per_second > MANY_PER_SECOND {
                let top: Vec<String> = ranked.iter().take(3).map(|(cause, count)| format!("{cause} x{count}")).collect();
                tracing::warn!(
                    "the window is drawn {:.0} times a second ({:.0}% of the time drawing){}; asked for by: {}",
                    self.per_second,
                    self.busy_share * 100.,
                    if self.throttled { ", held back" } else { "" },
                    top.join("; ")
                );
            }
            self.since = now;
            self.frames = 0;
            self.busy = Duration::ZERO;
            self.causes.clear();
        }
    }
}
