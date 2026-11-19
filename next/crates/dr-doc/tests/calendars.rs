//! Calendar records merge as whole valid rules, independently by calendar id.
use dr_doc::v3::BookDocV3;
use dr_model::{Origin, calendar::CalendarSpec, id::BookId};

#[test]
fn concurrent_calendar_rules_converge_without_mixing_incompatible_fields() {
    let book = BookId::parse("synthetic-calendar-book").unwrap();
    let mut first = BookDocV3::create(book.clone(), 101).unwrap();
    let mut cal = CalendarSpec::simple("dawn".into(), "Dawn".into());
    first
        .edit(Origin::User, |tx| {
            tx.project().calendar(&cal.id, Some(&cal))?;
            Ok(())
        })
        .0
        .unwrap();
    let mut second = BookDocV3::create(book, 102).unwrap();
    second
        .import(&first.export_snapshot().unwrap(), Origin::Sync)
        .unwrap();
    cal.months[0].days = 31;
    first
        .edit(Origin::User, |tx| {
            tx.project().calendar("dawn", Some(&cal))?;
            Ok(())
        })
        .0
        .unwrap();
    let mut alternative = cal.clone();
    alternative.months[0].days = 45;
    alternative.name = "Other Dawn".into();
    let unrelated = CalendarSpec::simple("another".into(), "Another".into());
    second
        .edit(Origin::User, |tx| {
            tx.project().calendar("dawn", Some(&alternative))?;
            tx.project().calendar("another", Some(&unrelated))?;
            Ok(())
        })
        .0
        .unwrap();
    let a = first.export_snapshot().unwrap();
    let b = second.export_snapshot().unwrap();
    first.import(&b, Origin::Sync).unwrap();
    second.import(&a, Origin::Sync).unwrap();
    assert_eq!(
        first.view().project().calendars,
        second.view().project().calendars
    );
    let result = first.view().calendar("dawn").unwrap().unwrap();
    assert!(result == cal || result == alternative);
    assert_eq!(first.view().calendar("another").unwrap(), Some(unrelated));
}
