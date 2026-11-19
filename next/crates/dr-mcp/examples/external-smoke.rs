//! Opt-in live acceptance: installed external harness -> real MCP -> synthetic book.
//! No user library or global harness configuration is changed.
use dr_agent::{Options, Runtime, driver::scripted::ScriptedDriver};
use dr_agent_tools::AgentTools;
use dr_mcp::{
    ClientPaths, GrantPaths, GrantStore,
    host::{Backend, Host},
    inbound::Server,
    transport::Listener,
};
use dr_platform::external_agent::{
    ExternalCommand, ExternalPlatform, ExternalSession, McpCommand, Wire,
};
use dr_testkit::seed::{self, Preset};
use dr_workspace::{Actor, Workspace};
use futures::FutureExt;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let dr_platform::cli::Launch::Mcp(path) = dr_platform::cli::parse(&args) {
        return Ok(dr_mcp::bridge::run(&path?)?);
    }
    let platform = match args.first().map(String::as_str) {
        Some("codex") => ExternalPlatform::Codex,
        Some("dsh") => ExternalPlatform::DeepseekHarness,
        _ => {
            return Err(
                "usage: external-smoke codex|dsh (uses the installed harness's own login)".into(),
            );
        }
    };
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run(platform))
}

async fn receive(process: &mut ExternalSession) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Some(event) = process.try_recv() {
            return Ok(event?);
        }
        if Instant::now() >= deadline {
            return Err("external harness timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn run(platform: ExternalPlatform) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let ws = Workspace::open_dir(root.join("library"), "synthetic-external-smoke")?;
    ws.set_auto_links(false);
    let data = seed::generate_v3(Preset::Tiny, seed::DEFAULT_SEED)?;
    let book = data.books[0].book_id().clone();
    data.write_to(ws.library())?;
    let other = seed::generate_v3(Preset::Tiny, seed::DEFAULT_SEED + 1)?;
    let other_book = other.books[0].book_id().clone();
    other.write_to(ws.library())?;
    let other_before = ws.call(
        &Actor::User,
        "entry.list",
        json!({"book":other_book,"all":true}),
    )?;
    let tools = AgentTools::new();
    let agent = Runtime::new(
        ws.clone(),
        Arc::new(ScriptedDriver::new(vec![])),
        Arc::new(tools.clone()),
        Options::default(),
    )?;
    let paths = GrantPaths::new(
        root.join("mcp"),
        std::env::current_exe()?,
        ClientPaths::from_home(root.join("unused-home"), None, None),
    );
    let listener = Listener::loopback().await?;
    let grants = GrantStore::open(paths, listener.endpoint()?)?;
    let host = Host::new(tokio::runtime::Handle::current());
    let opening = agent.clone();
    let catalog_ws = ws.clone();
    host.replace(
        Some(book.clone()),
        Some(Backend {
            opener: Arc::new(move |start| {
                let agent = opening.clone();
                async move {
                    tokio::task::spawn_blocking(move || agent.open_external(start))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())
                }
                .boxed()
            }),
            catalog: Arc::new(move |book| {
                let (tools, ws) = (tools.clone(), catalog_ws.clone());
                async move {
                    tokio::task::spawn_blocking(move || tools.catalog_for_book(&ws, &book))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())
                }
                .boxed()
            }),
        }),
    );
    let follow = host.follow(&ws);
    let server = Server::start(
        listener,
        grants.clone(),
        host.clone(),
        &tokio::runtime::Handle::current(),
    );
    let grant = grants
        .create_for_book("Synthetic live harness acceptance", book.as_str())
        .await?;
    let mcp = McpCommand {
        command: grant.config.command.to_string_lossy().into_owned(),
        args: grant.config.args,
    };
    let wire = Wire { platform };
    let command = match platform {
        ExternalPlatform::Codex => ExternalCommand::codex(),
        ExternalPlatform::DeepseekHarness => ExternalCommand::deepseek_harness(),
    };
    let mut process = ExternalSession::spawn(&command)?;
    process.send(wire.initialize())?;
    loop {
        let message = receive(&mut process).await?;
        if message.get("error").is_some() {
            return Err("external initialize rejected".into());
        }
        if message["id"] == 1 {
            break;
        }
    }
    if platform == ExternalPlatform::Codex {
        process.send(json!({"jsonrpc":"2.0","method":"initialized"}))?;
    }
    process.send(wire.open(
        root.to_str().ok_or("non-UTF8 temporary directory")?,
        None,
        &mcp,
    )?)?;
    let session = loop {
        let message = receive(&mut process).await?;
        if message.get("error").is_some() {
            return Err(
                "external session rejected; check the harness's own login/configuration".into(),
            );
        }
        if let Some(id) = wire.session_id(&message) {
            break id;
        }
    };
    println!("{platform:?}: initialize and scoped MCP session established");
    let title = "Synthetic live harness entry";
    process.send(wire.prompt(11, &session, &format!("This is an explicit automated acceptance test in a temporary synthetic Drifting library. Use only the drifting MCP tools, never shell commands or filesystem tools. Inspect the open book {book} and its template catalog, then use entry.create to create one entry with title '{title}' using the existing tpl-idea template, with no parent. Do not modify other entries or books. Finish after the tool reports success.")))?;
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        if Instant::now() >= deadline {
            return Err("live writing turn timed out".into());
        }
        let message = receive(&mut process).await?;
        if message.get("error").is_some() {
            return Err(format!(
                "external turn error ({}): {}",
                message["error"]["code"],
                dr_platform::sanitize::message(
                    message["error"]["message"]
                        .as_str()
                        .unwrap_or("request rejected")
                )
            )
            .into());
        }
        if message["method"] == "item/completed" {
            let item = &message["params"]["item"];
            if item["type"] == "mcpToolCall" {
                println!("MCP tool {}: {}", item["tool"], item["status"]);
                if !item["error"].is_null() {
                    println!(
                        "MCP error: {}",
                        dr_platform::sanitize::message(&item["error"].to_string())
                    );
                }
                if item["status"] == "failed"
                    && let Some(content) = item["result"]["content"].as_array()
                {
                    for block in content {
                        if let Some(text) = block["text"].as_str() {
                            println!("MCP refusal: {}", dr_platform::sanitize::message(text));
                        }
                    }
                }
            } else if item["type"] == "agentMessage" {
                println!(
                    "External answer: {}",
                    dr_platform::sanitize::message(item["text"].as_str().unwrap_or_default())
                );
            }
        }
        if message.get("id").is_some() && message.get("method").is_some() {
            let method = message["method"].as_str().unwrap_or_default();
            let result = if method == "session/request_permission" {
                let option = message["params"]["options"]
                    .as_array()
                    .and_then(|options| options.iter().find(|o| o["kind"] == "allow_once"))
                    .ok_or("harness requested an unsupported permission")?;
                json!({"outcome":{"outcome":"selected","optionId":option["optionId"]}})
            } else if method == "mcpServer/elicitation/request" {
                // This run explicitly authorizes one synthetic MCP write, but
                // never logs into unrelated globally configured integrations.
                if message["params"]["serverName"] == "drifting"
                    && message["params"]["mode"] == "form"
                {
                    json!({"action":"accept","content":{}})
                } else {
                    json!({"action":"decline"})
                }
            } else if method.ends_with("requestApproval") {
                json!({"decision":"decline"})
            } else {
                return Err(
                    format!("harness requested unsupported interactive method: {method}").into(),
                );
            };
            process.send(json!({"jsonrpc":"2.0","id":message["id"],"result":result}))?;
        }
        if let Some(result) = wire.completion(&message, Some(11)) {
            result.map_err(|status| format!("external writing turn ended: {status}"))?;
            break;
        }
    }
    let rows = ws.call(&Actor::User, "entry.list", json!({"book":book,"all":true}))?;
    assert!(
        rows["output"]
            .as_array()
            .ok_or("entry list missing")?
            .iter()
            .any(|row| row["title"] == title),
        "the real harness must write through MCP"
    );
    assert!(
        !ws.library().settlements(None, None)?.is_empty(),
        "a durable writing receipt is required"
    );
    assert_eq!(
        other_before,
        ws.call(
            &Actor::User,
            "entry.list",
            json!({"book":other_book,"all":true})
        )?,
        "the other synthetic book is unchanged"
    );
    grants.revoke(&grant.grant.id).await?;
    drop(process);
    host.suspend();
    server.close().await?;
    drop(follow);
    agent.drain_tools().await?;
    grants.drain().await?;
    drop(server);
    drop(host);
    drop(grants);
    drop(agent);
    ws.try_shutdown()
        .map_err(|error| format!("synthetic library still in use: {error:?}"))?;
    println!(
        "{platform:?}: real MCP write, durable receipt, book isolation and grant revocation passed"
    );
    Ok(())
}
