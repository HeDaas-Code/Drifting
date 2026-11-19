use dr_agent_tools::{AgentTools, CallRequest, Error};
use dr_model::{calendar::CalendarSpec, id::BookId};
use dr_workspace::{Actor, Workspace};
use serde_json::{Value, json};

#[test]
fn calendar_tools_require_current_book_read_and_reject_stale_rules() {
    let temp = tempfile::tempdir().unwrap();
    let ws = Workspace::open_dir(temp.path(), "test").unwrap();
    let book: BookId = ws
        .call(
            &Actor::User,
            "project.create",
            json!({"name":"Synthetic Agent calendar"}),
        )
        .unwrap()["output"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let tools = AgentTools::new();
    let invoke = |name: &str, arguments: Value| {
        tools.invoke(
            &ws,
            &book,
            &Actor::agent("calendar-test", 1, name),
            CallRequest {
                name: name.into(),
                arguments,
                revision: None,
                edited_text: None,
            },
        )
    };
    let mut spec = CalendarSpec::simple("dawn".into(), "Dawn".into());
    assert!(matches!(
        invoke("calendar.put", json!({"calendar":spec})),
        Err(Error::ReadRequired(_))
    ));
    invoke("calendar.list", json!({})).unwrap();
    invoke("calendar.put", json!({"calendar":spec})).unwrap();
    let shown = invoke("calendar.list", json!({})).unwrap();
    assert_eq!(shown["output"][0]["name"], "Dawn");
    spec.months[0].days = 42;
    ws.call(
        &Actor::User,
        "calendar.put",
        json!({"book":book,"calendar":spec}),
    )
    .unwrap();
    spec.months[0].days = 55;
    assert!(invoke("calendar.put", json!({"calendar":spec})).is_err());
    let original = ws.observe(&format!("book:{book}")).unwrap().version;
    assert!(invoke("calendar.list", json!({"book":"another-book"})).is_err());
    assert_eq!(
        ws.observe(&format!("book:{book}")).unwrap().version,
        original
    );
    ws.close();
}
