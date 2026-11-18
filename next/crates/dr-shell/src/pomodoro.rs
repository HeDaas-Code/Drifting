//! Writing-focused Pomodoro state. The UI can poll this value; elapsed time is
//! derived from a deadline so sleep and app restarts do not reset a session.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub focus_minutes: u32,
    pub short_break_minutes: u32,
    pub long_break_minutes: u32,
    pub rounds_before_long_break: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            rounds_before_long_break: 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    pub book_id: String,
    pub phase: Phase,
    pub round: u32,
    pub deadline_ms: i64,
    pub paused_remaining_ms: Option<i64>,
    pub muted: bool,
}

impl Timer {
    pub fn start(book_id: impl Into<String>, now_ms: i64, settings: Settings) -> Self {
        Self {
            book_id: book_id.into(),
            phase: Phase::Focus,
            round: 1,
            deadline_ms: now_ms + i64::from(settings.focus_minutes) * 60_000,
            paused_remaining_ms: None,
            muted: false,
        }
    }

    pub fn pause(mut self, now_ms: i64) -> Self {
        self.paused_remaining_ms = Some((self.deadline_ms - now_ms).max(0));
        self
    }

    pub fn resume(mut self, now_ms: i64) -> Self {
        if let Some(remaining) = self.paused_remaining_ms.take() {
            self.deadline_ms = now_ms + remaining;
        }
        self
    }

    pub fn remaining_ms(&self, now_ms: i64) -> i64 {
        self.paused_remaining_ms
            .unwrap_or_else(|| (self.deadline_ms - now_ms).max(0))
    }

    pub fn complete(mut self, now_ms: i64, settings: Settings) -> Self {
        if self.phase == Phase::Focus {
            self.round += 1;
        }
        self.phase = if self.phase != Phase::Focus {
            Phase::Focus
        } else if self.round > settings.rounds_before_long_break {
            Phase::LongBreak
        } else {
            Phase::ShortBreak
        };
        let minutes = match self.phase {
            Phase::Focus => settings.focus_minutes,
            Phase::ShortBreak => settings.short_break_minutes,
            Phase::LongBreak => settings.long_break_minutes,
        };
        self.deadline_ms = now_ms + i64::from(minutes) * 60_000;
        self.paused_remaining_ms = None;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_resume_uses_deadline_time_and_survives_sleep() {
        let settings = Settings::default();
        let timer = Timer::start("book", 0, settings);
        assert_eq!(timer.remaining_ms(60_000), 1_440_000);
        let paused = timer.pause(60_000);
        assert_eq!(paused.remaining_ms(3_600_000), 1_440_000);
        assert_eq!(paused.resume(3_600_000).remaining_ms(3_600_000), 1_440_000);
    }

    #[test]
    fn fourth_focus_round_enters_long_break_then_focus() {
        let settings = Settings::default();
        let mut timer = Timer::start("book", 0, settings);
        for _ in 0..3 {
            timer = timer.complete(0, settings);
            timer = timer.complete(0, settings);
        }
        timer = timer.complete(0, settings);
        assert_eq!(timer.phase, Phase::LongBreak);
        timer = timer.complete(0, settings);
        assert_eq!(timer.phase, Phase::Focus);
    }
}
