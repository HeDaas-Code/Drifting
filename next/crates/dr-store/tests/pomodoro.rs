use dr_store::{Library, LibraryOptions, PomodoroCompletion, PomodoroSummary};

#[test]
fn focus_sessions_are_idempotent_and_survive_library_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let options = LibraryOptions::new("test");
    let library = Library::open(dir.path(), options.clone()).unwrap();
    let completion = PomodoroCompletion {
        id: "session-a:focus:1".into(),
        book_id: "book-a".into(),
        local_day: "2026-11-19".into(),
        started_ms: 1_000,
        completed_ms: 1_501_000,
        focus_minutes: 25,
    };

    assert!(library.record_pomodoro(&completion).unwrap());
    assert!(!library.record_pomodoro(&completion).unwrap());
    let mut conflicting_retry = completion.clone();
    conflicting_retry.book_id = "book-b".into();
    assert!(library.record_pomodoro(&conflicting_retry).is_err());
    drop(library);

    let library = Library::open(dir.path(), options).unwrap();
    assert_eq!(
        library.pomodoro_summary("book-a", "2026-11-19").unwrap(),
        PomodoroSummary {
            today_sessions: 1,
            total_sessions: 1,
            total_focus_minutes: 25,
        }
    );
    assert_eq!(
        library
            .pomodoro_summary("book-a", "2026-11-20")
            .unwrap()
            .today_sessions,
        0
    );
    assert_eq!(
        library
            .pomodoro_summary("book-b", "2026-11-19")
            .unwrap()
            .total_sessions,
        0
    );
}

#[test]
fn invalid_local_date_and_timer_values_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(dir.path(), LibraryOptions::new("test")).unwrap();
    let mut completion = PomodoroCompletion {
        id: "session-a:focus:1".into(),
        book_id: "book-a".into(),
        local_day: "2026-13-40".into(),
        started_ms: 1_000,
        completed_ms: 1_001,
        focus_minutes: 25,
    };
    assert!(library.record_pomodoro(&completion).is_err());
    completion.local_day = "2026-11-19".into();
    completion.focus_minutes = 181;
    assert!(library.record_pomodoro(&completion).is_err());
}
