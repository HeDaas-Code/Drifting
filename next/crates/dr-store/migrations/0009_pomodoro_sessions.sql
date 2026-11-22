CREATE TABLE pomodoro_session (
    completion_id TEXT PRIMARY KEY CHECK (length(completion_id) > 0),
    book_id TEXT NOT NULL CHECK (length(book_id) > 0),
    local_day TEXT NOT NULL CHECK (length(local_day) = 10),
    started_ms INTEGER NOT NULL,
    completed_ms INTEGER NOT NULL,
    focus_minutes INTEGER NOT NULL CHECK (focus_minutes BETWEEN 1 AND 180),
    CHECK (completed_ms >= started_ms)
) STRICT, WITHOUT ROWID;

CREATE INDEX pomodoro_session_book_day
    ON pomodoro_session (book_id, local_day);
