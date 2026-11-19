mod common;
use common::*;
use dr_model::calendar::{CalendarSpec, Calibration};
use dr_workspace::Actor;
use serde_json::json;

#[test]
fn calendars_and_explicit_view_calibration_survive_reopen_without_changing_positions() {
    let mut env = env();
    let book = env.out(
        "project.create",
        json!({"name":"Synthetic calendar project"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let calendar = CalendarSpec::simple("dawn".into(), "Dawn".into());
    env.call("calendar.put", json!({"book":book,"calendar":calendar}));
    env.call("timeline_view.put", json!({"book":book,"view":{"id":"calendar-view","name":"Timeline","pins":[{"id":"pin","label":"Synthetic event","at":-2.5}]}}));
    let original = env.out("timeline_view.list", json!({"book":book}));
    let version = env.ws().observe(&format!("book:{book}")).unwrap().version;
    assert!(
        env.ws()
            .call(
                &Actor::User,
                "timeline_view.calendar",
                json!({"book":book,"view":"calendar-view","calendar":"dawn"})
            )
            .is_err()
    );
    assert_eq!(
        env.ws().observe(&format!("book:{book}")).unwrap().version,
        version
    );
    let mapping = Calibration {
        coordinate: -2.5,
        day: -100,
        days_per_unit: 2.,
    };
    env.call(
        "timeline_view.calendar",
        json!({"book":book,"view":"calendar-view","calendar":"dawn","calibration":mapping}),
    );
    let mut other = CalendarSpec::simple("before".into(), "Before".into());
    other.epoch_day = 1000;
    other.months[0].days = 50;
    env.call("calendar.put", json!({"book":book,"calendar":other}));
    env.call(
        "timeline_view.calendar",
        json!({"book":book,"view":"calendar-view","calendar":"before"}),
    );
    let current = env.out("timeline_view.list", json!({"book":book}));
    let read = |views: serde_json::Value| {
        views
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == "calendar-view")
            .unwrap()
            .clone()
    };
    assert_eq!(read(original)["pins"], read(current.clone())["pins"]);
    assert_eq!(read(current)["calendar_calibration"], json!(mapping));
    assert!(
        env.ws()
            .call(
                &Actor::User,
                "calendar.delete",
                json!({"book":book,"id":"before"})
            )
            .is_err()
    );
    assert!(env.ws().call(&Actor::User,"timeline_view.calendar",json!({"book":book,"view":"calendar-view","calendar":"before","calibration":{"coordinate":0,"day":0,"days_per_unit":1}})).is_err());
    env.reopen();
    assert_eq!(
        env.out("calendar.list", json!({"book":book}))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        read(env.out("timeline_view.list", json!({"book":book})))["calendar_id"],
        "before"
    );
}

#[test]
fn calendar_edit_undo_and_stale_draft_preserve_other_calendars() {
    let env = env();
    let book = env.out("project.create", json!({"name":"Synthetic calendar undo"}))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = CalendarSpec::simple("one".into(), "One".into());
    env.call("calendar.put", json!({"book":book,"calendar":original}));
    let other = CalendarSpec::simple("two".into(), "Two".into());
    env.call("calendar.put", json!({"book":book,"calendar":other}));
    let mut edited = original.clone();
    edited.months[0].days = 47;
    env.call(
        "calendar.restore",
        json!({"book":book,"id":"one","expected":original,"value":edited}),
    );
    assert!(
        env.ws()
            .call(
                &Actor::User,
                "calendar.restore",
                json!({"book":book,"id":"one","expected":original,"value":original})
            )
            .is_err()
    );
    env.ws().undo(&Actor::User).unwrap().unwrap();
    let list: Vec<CalendarSpec> =
        serde_json::from_value(env.out("calendar.list", json!({"book":book}))).unwrap();
    assert!(list.contains(&original));
    assert!(list.contains(&other));
}
