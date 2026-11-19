//! Writing-focused Pomodoro state. The UI can poll this value; elapsed time is
//! derived from a deadline so sleep and app restarts do not reset a session.

use serde::{Deserialize, Serialize};

use crate::Settings as ShellSettings;
use gpui::{App, BorrowAppContext, Global};

const PERSISTED_KEY: &str = "pomodoro.v1";

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
    pub session_id: String,
    pub book_id: String,
    #[serde(default)]
    library_id: String,
    #[serde(default)]
    pub book_name: String,
    pub phase: Phase,
    pub round: u32,
    pub focus_sequence: u64,
    pub deadline_ms: i64,
    pub phase_started_ms: i64,
    pub phase_minutes: u32,
    pub paused_remaining_ms: Option<i64>,
    pub muted: bool,
}

pub struct Controller {
    pub timer: Option<Timer>,
    pub settings: Settings,
    pub panel_open: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Persisted {
    version: u8,
    timer: Option<Timer>,
    settings: Settings,
}

impl Global for Controller {}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn save(cx: &mut App) {
    let Some(controller) = cx.try_global::<Controller>() else {
        return;
    };
    let persisted = Persisted {
        version: 1,
        timer: controller.timer.clone(),
        settings: controller.settings,
    };
    let Some(value) = serde_json::to_value(persisted).ok() else {
        return;
    };
    ShellSettings::set_device_memory(cx, PERSISTED_KEY, Some(value));
    ShellSettings::flush(cx);
}

pub fn install(cx: &mut App) {
    let persisted = ShellSettings::device_memory(cx, PERSISTED_KEY)
        .and_then(|value| serde_json::from_value::<Persisted>(value.clone()).ok())
        .filter(|persisted| persisted.version == 1)
        .unwrap_or_default();
    cx.set_global(Controller {
        timer: persisted.timer,
        settings: persisted.settings.validated(),
        panel_open: false,
    });
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            cx.update(tick);
        }
    })
    .detach();
}

pub fn toggle_panel(cx: &mut App) {
    cx.update_global::<Controller, _>(|controller, _| {
        controller.panel_open = !controller.panel_open;
    });
    cx.refresh_windows();
}

pub fn panel_open(cx: &App) -> bool {
    cx.try_global::<Controller>().is_some_and(|c| c.panel_open)
}

pub fn start(book_id: &str, book_name: &str, cx: &mut App) {
    if book_id.is_empty() || cx.global::<Controller>().timer.is_some() {
        return;
    }
    let Some(library_id) = cx
        .try_global::<crate::Services>()
        .and_then(|services| services.library.as_ref())
        .map(|workspace| workspace.library().library_id().to_owned())
    else {
        return;
    };
    let settings = cx.global::<Controller>().settings;
    cx.update_global::<Controller, _>(|controller, _| {
        let mut timer = Timer::start(book_id, now_ms(), settings);
        timer.book_name = book_name.to_owned();
        timer.library_id = library_id;
        controller.timer = Some(timer);
    });
    save(cx);
    cx.refresh_windows();
}

pub fn pause(cx: &mut App) {
    if cx
        .try_global::<Controller>()
        .and_then(|controller| controller.timer.as_ref())
        .is_some_and(|timer| timer.paused_remaining_ms.is_some())
    {
        return;
    }
    let before = cx
        .try_global::<Controller>()
        .and_then(|controller| controller.timer.clone());
    if !tick(cx) || expired_timer_is_pending(cx) {
        return;
    }
    if before
        != cx
            .try_global::<Controller>()
            .and_then(|controller| controller.timer.clone())
    {
        return;
    }
    let now = now_ms();
    cx.update_global::<Controller, _>(|controller, _| {
        if let Some(timer) = controller.timer.take() {
            controller.timer = Some(timer.pause(now));
        }
    });
    save(cx);
    cx.refresh_windows();
}

pub fn resume(cx: &mut App) {
    let now = now_ms();
    cx.update_global::<Controller, _>(|controller, _| {
        if let Some(timer) = controller.timer.take() {
            controller.timer = Some(timer.resume(now));
        }
    });
    save(cx);
    cx.refresh_windows();
}

pub fn stop(cx: &mut App) {
    if !tick(cx) || expired_timer_is_pending(cx) {
        return;
    }
    cx.update_global::<Controller, _>(|controller, _| controller.timer = None);
    save(cx);
    cx.refresh_windows();
}

pub fn toggle_mute(cx: &mut App) {
    cx.update_global::<Controller, _>(|controller, _| {
        if let Some(timer) = controller.timer.as_mut() {
            timer.muted = !timer.muted;
        }
    });
    save(cx);
    cx.refresh_windows();
}

pub fn adjust_minutes(phase: Phase, delta: i32, cx: &mut App) {
    cx.update_global::<Controller, _>(|controller, _| {
        let value = match phase {
            Phase::Focus => &mut controller.settings.focus_minutes,
            Phase::ShortBreak => &mut controller.settings.short_break_minutes,
            Phase::LongBreak => &mut controller.settings.long_break_minutes,
        };
        *value = (i32::try_from(*value).unwrap_or(1) + delta).clamp(1, 180) as u32;
    });
    save(cx);
    cx.refresh_windows();
}

pub fn tick(cx: &mut App) -> bool {
    let now = now_ms();
    let Some(timer) = cx
        .try_global::<Controller>()
        .and_then(|controller| controller.timer.clone())
    else {
        return true;
    };
    if !timer.is_due(now) {
        if panel_open(cx) && timer.paused_remaining_ms.is_none() {
            cx.refresh_windows();
        }
        return true;
    }
    let settings = cx.global::<Controller>().settings;
    let (timer, completed) = advance_due(timer, now, settings);
    let recorded = if let Some(completed) = completed {
        let Some(workspace) = cx
            .try_global::<crate::Services>()
            .and_then(|services| services.library.as_ref())
            .filter(|workspace| workspace.library().library_id() == timer.library_id)
        else {
            return false;
        };
        workspace.record_pomodoro(&completed)
    } else {
        Ok(false)
    };
    let inserted = match recorded {
        Ok(inserted) => inserted,
        Err(error) => {
            log::error!("could not record completed focus sessions: {error}");
            cx.refresh_windows();
            return false;
        }
    };
    let ring = inserted && !timer.muted;
    cx.update_global::<Controller, _>(|controller, _| {
        if controller.timer.as_ref().is_some_and(|stored| {
            stored.session_id == timer.session_id
                && stored.focus_sequence <= timer.focus_sequence
                && stored.deadline_ms <= now
        }) {
            controller.timer = Some(timer.clone());
        }
    });
    save(cx);
    if ring && let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_, window, _| window.play_system_bell());
    }
    cx.refresh_windows();
    true
}

// Finish the phase the writer left running. A break may have elapsed too,
// but the next focus starts on return, so time away never earns extra rounds.
fn advance_due(
    mut timer: Timer,
    now: i64,
    settings: Settings,
) -> (Timer, Option<dr_store::PomodoroCompletion>) {
    if !timer.is_due(now) {
        return (timer, None);
    }
    let mut completed = None;
    if timer.phase == Phase::Focus {
        completed = Some(completion(&timer));
        let deadline = timer.deadline_ms;
        timer = timer.complete(deadline, settings);
    }
    if timer.is_due(now) {
        timer = timer.complete(now, settings);
    }
    (timer, completed)
}

fn expired_timer_is_pending(cx: &App) -> bool {
    cx.try_global::<Controller>()
        .and_then(|controller| controller.timer.as_ref())
        .is_some_and(|timer| timer.paused_remaining_ms.is_none() && timer.deadline_ms <= now_ms())
}

fn completion(timer: &Timer) -> dr_store::PomodoroCompletion {
    let completed_at = chrono::DateTime::from_timestamp_millis(timer.deadline_ms)
        .unwrap_or_else(chrono::Utc::now)
        .with_timezone(&chrono::Local);
    dr_store::PomodoroCompletion {
        id: format!(
            "{}:focus:{}",
            timer.session_id,
            timer.focus_sequence.saturating_add(1)
        ),
        book_id: timer.book_id.clone(),
        local_day: completed_at.format("%Y-%m-%d").to_string(),
        started_ms: timer.phase_started_ms,
        completed_ms: timer.deadline_ms,
        focus_minutes: timer.phase_minutes,
    }
}

pub fn remaining_ms(cx: &App) -> Option<i64> {
    cx.try_global::<Controller>()
        .and_then(|controller| controller.timer.as_ref())
        .map(|timer| timer.remaining_ms(now_ms()))
}

impl Timer {
    pub fn start(book_id: impl Into<String>, now_ms: i64, settings: Settings) -> Self {
        let book_id = book_id.into();
        Self {
            session_id: uuid::Uuid::now_v7().to_string(),
            book_name: book_id.clone(),
            library_id: String::new(),
            book_id,
            phase: Phase::Focus,
            round: 1,
            focus_sequence: 0,
            deadline_ms: now_ms + i64::from(settings.focus_minutes) * 60_000,
            phase_started_ms: now_ms,
            phase_minutes: settings.focus_minutes,
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

    fn is_due(&self, now_ms: i64) -> bool {
        self.paused_remaining_ms.is_none() && self.deadline_ms <= now_ms
    }

    pub fn remaining_ms(&self, now_ms: i64) -> i64 {
        self.paused_remaining_ms
            .unwrap_or_else(|| (self.deadline_ms - now_ms).max(0))
    }

    pub fn complete(mut self, now_ms: i64, settings: Settings) -> Self {
        let completed_phase = self.phase;
        self.phase = match completed_phase {
            Phase::Focus if self.round >= settings.rounds_before_long_break => Phase::LongBreak,
            Phase::Focus => Phase::ShortBreak,
            Phase::ShortBreak | Phase::LongBreak => Phase::Focus,
        };
        match completed_phase {
            Phase::Focus => {
                self.focus_sequence = self.focus_sequence.saturating_add(1);
                if self.phase == Phase::ShortBreak {
                    self.round = self.round.saturating_add(1);
                }
            }
            Phase::LongBreak => self.round = 1,
            Phase::ShortBreak => {}
        }
        let minutes = match self.phase {
            Phase::Focus => settings.focus_minutes,
            Phase::ShortBreak => settings.short_break_minutes,
            Phase::LongBreak => settings.long_break_minutes,
        };
        self.phase_started_ms = now_ms;
        self.phase_minutes = minutes;
        self.deadline_ms = now_ms + i64::from(minutes) * 60_000;
        self.paused_remaining_ms = None;
        self
    }
}

impl Settings {
    fn validated(self) -> Self {
        Self {
            focus_minutes: self.focus_minutes.clamp(1, 180),
            short_break_minutes: self.short_break_minutes.clamp(1, 180),
            long_break_minutes: self.long_break_minutes.clamp(1, 180),
            rounds_before_long_break: self.rounds_before_long_break.clamp(1, 12),
        }
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
        assert_eq!(timer.round, 1);
    }

    #[test]
    fn every_fourth_focus_round_gets_one_long_break_and_the_round_resets() {
        let settings = Settings::default();
        let mut timer = Timer::start("book", 0, settings);
        for cycle in 0..3 {
            for _ in 0..3 {
                assert_eq!(timer.phase, Phase::Focus);
                timer = timer.complete(0, settings);
                assert_eq!(timer.phase, Phase::ShortBreak);
                timer = timer.complete(0, settings);
            }
            assert_eq!(timer.round, 4);
            timer = timer.complete(0, settings);
            assert_eq!(timer.phase, Phase::LongBreak);
            timer = timer.complete(0, settings);
            assert_eq!(timer.phase, Phase::Focus);
            assert_eq!(timer.round, 1, "cycle {cycle}");
            assert_eq!(timer.focus_sequence, (cycle as u64 + 1) * 4);
        }
        assert_eq!(
            completion(&timer).id,
            format!("{}:focus:13", timer.session_id)
        );
    }

    #[test]
    fn timer_checkpoint_preserves_book_deadline_pause_and_session_id() {
        let settings = Settings::default();
        let timer = Timer::start("source-book", 10_000, settings).pause(70_000);
        let encoded = serde_json::to_vec(&timer).unwrap();
        let resumed: Timer = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(resumed, timer);
        assert_eq!(resumed.book_id, "source-book");
        assert_eq!(resumed.remaining_ms(9_000_000), 1_440_000);
    }

    #[test]
    fn late_completion_retry_keeps_the_same_record_and_daily_bucket() {
        let timer = Timer::start("source-book", 1_764_000_000_000, Settings::default());
        let checkpoint = serde_json::to_vec(&timer).unwrap();
        let restored: Timer = serde_json::from_slice(&checkpoint).unwrap();
        let first = completion(&timer);
        let retry_after_sleep = completion(&restored);
        assert_eq!(first, retry_after_sleep);
        assert_eq!(first.completed_ms, timer.deadline_ms);
    }

    #[test]
    fn waking_after_a_day_records_one_round_and_starts_a_fresh_focus() {
        let settings = Settings::default();
        let now = 24 * 60 * 60 * 1_000;
        let timer = Timer::start("source-book", 0, settings);
        let (advanced, completions) = advance_due(timer, now, settings);
        let completion = completions.unwrap();
        assert_eq!(completion.book_id, "source-book");
        assert_eq!(completion.completed_ms, 25 * 60 * 1_000);
        assert_eq!(advanced.phase, Phase::Focus);
        assert_eq!(advanced.remaining_ms(now), 25 * 60 * 1_000);
        assert!(advance_due(advanced, now, settings).1.is_none());
    }
}
