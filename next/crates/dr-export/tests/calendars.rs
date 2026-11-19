use dr_export::book_file::{export_book, import_book};
use dr_model::calendar::*;
use dr_testkit::seed::{DEFAULT_SEED, Preset, generate_v3};
use dr_workspace::{Actor, Workspace};
use serde_json::json;

#[test]
fn book_files_preserve_calendars_calibration_and_event_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let lib = dr_store::Library::open(dir.path(), dr_workspace::library_options("test")).unwrap();
    let seed = generate_v3(Preset::Tiny, DEFAULT_SEED).unwrap();
    let book = seed.books[0].book_id().to_string();
    seed.write_to(&lib).unwrap();
    lib.close().unwrap();
    let ws = Workspace::open_dir(dir.path(), "test").unwrap();
    let mut calendar = CalendarSpec::simple("dawn".into(), "Dawn".into());
    calendar.intercalations.push(Intercalation {
        when: Periodic {
            every_years: 3,
            offset_year: -1,
            except_every: None,
            restore_every: None,
        },
        adjustment: LeapAdjustment::Month {
            after_month: 1,
            name: "Interlude".into(),
            days: 5,
        },
    });
    calendar.eras.push(Era {
        name: "First".into(),
        start_year: -10,
    });
    ws.call(
        &Actor::User,
        "calendar.put",
        json!({"book":book,"calendar":calendar}),
    )
    .unwrap();
    let before = ws
        .call(&Actor::User, "timeline_view.list", json!({"book":book}))
        .unwrap()["output"]
        .clone();
    let view = before[0]["id"].as_str().unwrap();
    let mapping = Calibration {
        coordinate: 0.,
        day: -100,
        days_per_unit: 0.5,
    };
    ws.call(
        &Actor::User,
        "timeline_view.calendar",
        json!({"book":book,"view":view,"calendar":"dawn","calibration":mapping}),
    )
    .unwrap();
    let file = dir.path().join("calendar.driftingproject");
    export_book(&ws, &book, &file, &|_| {}).unwrap();
    let copy = import_book(&ws, &file, &|_| {}).unwrap();
    let calendars: Vec<CalendarSpec> = serde_json::from_value(
        ws.call(&Actor::User, "calendar.list", json!({"book":copy.book}))
            .unwrap()["output"]
            .clone(),
    )
    .unwrap();
    assert_eq!(calendars, vec![calendar]);
    let after = ws
        .call(
            &Actor::User,
            "timeline_view.list",
            json!({"book":copy.book}),
        )
        .unwrap()["output"]
        .clone();
    assert_eq!(after[0]["calendar_id"], "dawn");
    assert_eq!(after[0]["calendar_calibration"], json!(mapping));
    let positions = |value: &serde_json::Value| {
        value["placements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["at"].as_f64().unwrap(), p["span"].as_f64()))
            .collect::<Vec<_>>()
    };
    assert_eq!(positions(&before[0]), positions(&after[0]));
    ws.shutdown().unwrap();
}

#[test]
fn project_templates_keep_calendar_rules_and_view_calibration() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open_dir(dir.path(), "test").unwrap();
    let book = ws
        .call(
            &Actor::User,
            "project.create",
            json!({"name":"Synthetic template calendar"}),
        )
        .unwrap()["output"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let spec = CalendarSpec::simple("dawn".into(), "Dawn".into());
    ws.call(
        &Actor::User,
        "calendar.put",
        json!({"book":book,"calendar":spec}),
    )
    .unwrap();
    let view = ws
        .call(&Actor::User, "timeline_view.list", json!({"book":book}))
        .unwrap()["output"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mapping = Calibration {
        coordinate: 2.,
        day: -10,
        days_per_unit: 0.5,
    };
    ws.call(
        &Actor::User,
        "timeline_view.calendar",
        json!({"book":book,"view":view,"calendar":"dawn","calibration":mapping}),
    )
    .unwrap();
    let template = ws
        .call(
            &Actor::User,
            "project_template.save",
            json!({"book":book,"name":"Calendar template"}),
        )
        .unwrap()["output"]["id"]
        .clone();
    let payload = ws
        .call(&Actor::User, "project_template.get", json!({"id":template}))
        .unwrap()["output"]["value"]
        .clone();
    assert_eq!(payload["project"]["calendars"][0]["id"], "dawn");
    assert_eq!(payload["views"][0]["calendar_id"], "dawn");
    assert_eq!(payload["views"][0]["calendar_calibration"], json!(mapping));
    ws.shutdown().unwrap();
}
