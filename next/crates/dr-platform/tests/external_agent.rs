//! Actual child processes and NDJSON pipes, with synthetic platform peers.
use dr_platform::external_agent::{
    ExternalCommand, ExternalPlatform, ExternalSession, McpCommand, Wire,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn receive(process: &mut ExternalSession) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = process.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "synthetic peer timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn both_platforms_stream_approve_complete_resume_cancel_and_report_disconnect() {
    for platform in [ExternalPlatform::Codex, ExternalPlatform::DeepseekHarness] {
        let command = ExternalCommand {
            platform,
            program: if cfg!(windows) { "python" } else { "python3" }.into(),
            args: vec![
                format!("{}/tests/fixtures/external.py", env!("CARGO_MANIFEST_DIR")),
                if platform == ExternalPlatform::Codex {
                    "codex"
                } else {
                    "dsh"
                }
                .into(),
            ],
        };
        let wire = Wire { platform };
        let mcp = McpCommand {
            command: "synthetic-writer".into(),
            args: vec![
                "--mcp".into(),
                "--connection".into(),
                "synthetic.json".into(),
            ],
        };
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().to_str().unwrap();
        let mut process = ExternalSession::spawn(&command).unwrap();
        process.send(wire.initialize()).unwrap();
        assert_eq!(receive(&mut process).unwrap()["id"], 1);
        process.send(wire.open(cwd, None, &mcp).unwrap()).unwrap();
        let session = wire.session_id(&receive(&mut process).unwrap()).unwrap();
        for round in 0..2 {
            let prompt = 11 + round;
            process
                .send(wire.prompt(prompt, &session, "synthetic prompt"))
                .unwrap();
            let mut chunks = String::new();
            loop {
                let message = receive(&mut process).unwrap();
                if message["id"] == 99 {
                    break;
                }
                if let Some(text) = message["params"]["delta"].as_str().or_else(|| {
                    message
                        .pointer("/params/update/content/text")
                        .and_then(Value::as_str)
                }) {
                    chunks.push_str(text);
                }
            }
            assert_eq!(chunks, "synthetic stream");
            if round == 0 {
                process
                    .send(json!({"id":99,"result":{"action":"accept","content":{}}}))
                    .unwrap();
            } else {
                process
                    .send(wire.cancel(20, &session, Some("synthetic-turn")).unwrap())
                    .unwrap();
            }
            let completed = receive(&mut process).unwrap();
            let result = wire
                .completion(&completed, Some(prompt))
                .expect("a terminal event");
            assert_eq!(result.is_ok(), round == 0);
            if round == 0 {
                process
                    .send(wire.open(cwd, Some(&session), &mcp).unwrap())
                    .unwrap();
                assert_eq!(
                    wire.session_id(&receive(&mut process).unwrap()),
                    Some(session.clone())
                );
            }
        }
        process
            .send(json!({"method":"fixture/disconnect"}))
            .unwrap();
        assert!(receive(&mut process).unwrap_err().contains("disconnected"));
    }
}
