//! Device-local, idempotent focus-session records, keyed to their source book.

use rusqlite::params;

use crate::Library;
use crate::error::{Result, StoreError};
use crate::events::{LocalChange, LocalChangeKind};

pub const TABLE: &str = "pomodoro_session";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PomodoroCompletion {
    pub id: String,
    pub book_id: String,
    pub local_day: String,
    pub started_ms: i64,
    pub completed_ms: i64,
    pub focus_minutes: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PomodoroSummary {
    pub today_sessions: u64,
    pub total_sessions: u64,
    pub total_focus_minutes: u64,
}

impl Library {
    /// Insert once by completion id. Replaying a due timer after a crash is harmless.
    pub fn record_pomodoro(&self, record: &PomodoroCompletion) -> Result<bool> {
        validate(record)?;
        self.local_write(|tx| {
            let changes = tx.execute(
                "INSERT INTO pomodoro_session \
                     (completion_id, book_id, local_day, started_ms, completed_ms, focus_minutes) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                     ON CONFLICT (completion_id) DO NOTHING",
                params![
                    record.id,
                    record.book_id,
                    record.local_day,
                    record.started_ms,
                    record.completed_ms,
                    record.focus_minutes,
                ],
            )?;
            if changes != 0 {
                tx.changed(LocalChange::new(TABLE, &record.id, LocalChangeKind::Insert));
            } else {
                let existing: (String, String, i64, i64, u32) = tx.query_row(
                    "SELECT book_id, local_day, started_ms, completed_ms, focus_minutes \
                         FROM pomodoro_session WHERE completion_id = ?1",
                    [&record.id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )?;
                let expected = (
                    record.book_id.clone(),
                    record.local_day.clone(),
                    record.started_ms,
                    record.completed_ms,
                    record.focus_minutes,
                );
                if existing != expected {
                    return Err(StoreError::invalid(
                        "pomodoro completion id",
                        "already belongs to a different completed session",
                    ));
                }
            }
            Ok(changes != 0)
        })
    }

    pub fn pomodoro_summary(&self, book_id: &str, local_day: &str) -> Result<PomodoroSummary> {
        if book_id.is_empty() || !valid_day(local_day) {
            return Err(StoreError::invalid(
                "pomodoro query",
                "invalid book or local day",
            ));
        }
        self.local_read(|conn| {
            let (today, total, minutes): (i64, i64, i64) = conn.query_row(
                "SELECT \
                   COALESCE(SUM(CASE WHEN local_day = ?2 THEN 1 ELSE 0 END), 0), \
                   COUNT(*), COALESCE(SUM(focus_minutes), 0) \
                 FROM pomodoro_session WHERE book_id = ?1",
                params![book_id, local_day],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            Ok(PomodoroSummary {
                today_sessions: today.max(0) as u64,
                total_sessions: total.max(0) as u64,
                total_focus_minutes: minutes.max(0) as u64,
            })
        })
    }
}

fn validate(record: &PomodoroCompletion) -> Result<()> {
    if record.id.is_empty()
        || record.book_id.is_empty()
        || !valid_day(&record.local_day)
        || record.completed_ms < record.started_ms
        || !(1..=180).contains(&record.focus_minutes)
    {
        return Err(StoreError::invalid("pomodoro completion", "invalid fields"));
    }
    Ok(())
}

fn valid_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, byte)| i != 4 && i != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let (Ok(year), Ok(month), Ok(date)) = (
        day[..4].parse::<i32>(),
        day[5..7].parse::<u32>(),
        day[8..].parse::<u32>(),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max).contains(&date)
}
