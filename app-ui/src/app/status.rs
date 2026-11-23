//! The status bar message.
//!
//! The bar shows one message at a time. It is for things the operator
//! can't already see elsewhere: errors, warnings, buttons that did
//! nothing, and the progress and result of long operations. Routine
//! confirmations (a QSO logged, the rig tuned) don't go here.

use std::time::{Duration, Instant};

use super::App;
use super::theme::Tone;

/// How long a message that expires stays up.
const SHOWN_FOR: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Level {
    /// Progress or the result of a long operation. Plain; expires.
    Info,
    /// A button did nothing, and why. Warning colour; expires.
    Notice,
    /// Something is wrong but nothing failed (a bad config value).
    /// Warning colour; stays until replaced.
    Warning,
    /// Something failed. Error colour; stays until replaced.
    Error,
}

#[derive(Debug, Clone)]
pub(super) struct Status {
    pub text: String,
    pub level: Level,
    since: Instant,
}

impl Status {
    pub(super) fn new(level: Level, text: impl Into<String>) -> Self {
        Self { text: text.into(), level, since: Instant::now() }
    }

    /// Whether the message should still be on screen at `now`.
    pub(super) fn shown_at(&self, now: Instant) -> bool {
        match self.level {
            Level::Info | Level::Notice => now.duration_since(self.since) < SHOWN_FOR,
            Level::Warning | Level::Error => true,
        }
    }

    /// The status colour the message is drawn in, if any.
    pub(super) fn tone(&self) -> Option<Tone> {
        match self.level {
            Level::Info => None,
            Level::Notice | Level::Warning => Some(Tone::Warn),
            Level::Error => Some(Tone::Bad),
        }
    }
}

impl App {
    /// Progress or the result of a long operation.
    pub(super) fn say(&mut self, text: impl Into<String>) {
        self.status = Some(Status::new(Level::Info, text));
    }

    /// A button did nothing, and why.
    pub(super) fn notice(&mut self, text: impl Into<String>) {
        self.status = Some(Status::new(Level::Notice, text));
    }

    /// Something is wrong but nothing failed.
    pub(super) fn warn(&mut self, text: impl Into<String>) {
        self.status = Some(Status::new(Level::Warning, text));
    }

    /// An action succeeded without needing a message of its own: takes
    /// down an earlier error or notice, which no longer describes how
    /// things stand. Warnings and information are left alone.
    pub(super) fn settle(&mut self) {
        if self.status.as_ref().is_some_and(|s| matches!(s.level, Level::Error | Level::Notice)) {
            self.status = None;
        }
    }

    /// Something failed.
    pub(super) fn fail(&mut self, text: impl Into<String>) {
        self.status = Some(Status::new(Level::Error, text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn information_and_notices_expire_and_problems_stay() {
        let later = Instant::now() + SHOWN_FOR + Duration::from_secs(1);
        for (level, stays) in [
            (Level::Info, false),
            (Level::Notice, false),
            (Level::Warning, true),
            (Level::Error, true),
        ] {
            let status = Status::new(level, "x");
            assert!(status.shown_at(Instant::now()), "{level:?} shows at first");
            assert_eq!(status.shown_at(later), stays, "{level:?}");
        }
    }

    #[test]
    fn only_information_is_uncoloured() {
        assert_eq!(Status::new(Level::Info, "x").tone(), None);
        assert_eq!(Status::new(Level::Notice, "x").tone(), Some(Tone::Warn));
        assert_eq!(Status::new(Level::Warning, "x").tone(), Some(Tone::Warn));
        assert_eq!(Status::new(Level::Error, "x").tone(), Some(Tone::Bad));
    }
}
