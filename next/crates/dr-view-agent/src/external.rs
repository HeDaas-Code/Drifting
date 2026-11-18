//! Sidebar client for locally configured Agent platforms. Backend-owned
//! credentials/models/language are never copied into the internal model router.
use dr_platform::external_agent::{
    ExternalCommand, ExternalPlatform, ExternalSession, McpCommand, Wire,
};
use dr_shell::Settings;
use dr_ui::{
    Theme,
    components::Button,
    dictation::{Dictation, InsertTranscript},
    input::TextArea,
};
use gpui::{prelude::*, *};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Saved {
    id: String,
    book: String,
    platform: ExternalPlatform,
    remote: Option<String>,
    messages: Vec<String>,
}
pub struct ExternalPanel {
    book: String,
    wire: Wire,
    saved: Saved,
    process: Option<ExternalSession>,
    grant: Option<(dr_mcp::GrantStore, String)>,
    pending_open: Option<(
        dr_mcp::grants::LaunchConfig,
        std::path::PathBuf,
        Option<String>,
    )>,
    opening: bool,
    ready: bool,
    running: bool,
    turn: Option<String>,
    serial: u64,
    request: Option<Value>,
    input: Entity<TextArea>,
    answer: Entity<TextArea>,
    dictation: Entity<Dictation>,
    status: String,
    scroll: ScrollHandle,
    generation: u64,
    _tick: Task<()>,
    _subs: Vec<Subscription>,
}
const SAVED: &str = "agent.external_sessions";

impl Drop for ExternalPanel {
    fn drop(&mut self) {
        self.process = None;
        if let Some((store, id)) = self.grant.take() {
            // GrantStore enqueues disk work synchronously even when the
            // returned future is dropped.
            drop(store.revoke(&id));
        }
    }
}
impl ExternalPanel {
    pub fn new(
        book: String,
        platform: ExternalPlatform,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let records: Vec<Saved> = Settings::device_memory(cx, SAVED)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let saved = records
            .into_iter()
            .rev()
            .find(|record| record.book == book && record.platform == platform)
            .unwrap_or_else(|| Saved {
                id: uuid::Uuid::new_v4().to_string(),
                book: book.clone(),
                platform,
                remote: None,
                messages: Vec::new(),
            });
        let input = cx.new(|cx| TextArea::new("external-input", window, cx).framed(3.0));
        let answer = cx.new(|cx| {
            TextArea::new("external-answer", window, cx)
                .framed(2.0)
                .placeholder("JSON response / JSON 答复")
        });
        let dictation = cx.new(|cx| Dictation::new(window, cx));
        let sub = cx.subscribe(&dictation, |this, _, event: &InsertTranscript, cx| {
            let value = format!("{}{}", this.input.read(cx).text(), event.0);
            this.input.update(cx, |field, cx| field.set_text(value, cx));
        });
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            book,
            wire: Wire { platform },
            saved,
            process: None,
            grant: None,
            pending_open: None,
            opening: false,
            ready: false,
            running: false,
            turn: None,
            serial: 10,
            request: None,
            input,
            answer,
            dictation,
            status: "Disconnected / 未连接".into(),
            scroll: ScrollHandle::new(),
            generation: 0,
            _tick: tick,
            _subs: vec![sub],
        }
    }
    fn save(&self, cx: &mut App) {
        let mut records: Vec<Saved> = Settings::device_memory(cx, SAVED)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        records.retain(|record| record.id != self.saved.id);
        records.push(self.saved.clone());
        if records.len() > 64 {
            records.drain(..records.len() - 64);
        }
        Settings::set_device_memory(cx, SAVED, serde_json::to_value(records).ok());
    }
    fn remember(&mut self, text: String, cx: &mut App) {
        self.saved.messages.push(text);
        if self.saved.messages.len() > 128 {
            self.saved.messages.remove(0);
        }
        self.save(cx);
    }
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.process = None;
        self.ready = false;
        self.running = false;
        self.opening = false;
        self.request = None;
        self.turn = None;
        self.dictation.update(cx, |view, cx| view.reset(cx));
        if let Some((store, id)) = self.grant.take() {
            drop(store.revoke(&id));
        }
        self.status = "Disconnected / 未连接".into();
        cx.notify();
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        self.disconnect(cx);
        let Some(store) = cx
            .try_global::<crate::mcp::McpGlobal>()
            .and_then(|global| global.service.read(cx).grants())
        else {
            self.status = "MCP service is not ready / MCP 服务尚未就绪".into();
            return;
        };
        let command = match self.wire.platform {
            ExternalPlatform::Codex => ExternalCommand::codex(),
            ExternalPlatform::DeepseekHarness => ExternalCommand::deepseek_harness(),
        };
        let book = self.book.clone();
        let generation = self.generation;
        self.opening = true;
        self.status = "Connecting / 连接中".into();
        let runtime = cx
            .global::<crate::AgentViews>()
            .connections
            .read(cx)
            .service()
            .driver()
            .http()
            .clone();
        let remote = self.saved.remote.clone();
        let wire = self.wire.clone();
        let id = self.saved.id.clone();
        let create = store.create_for_book("Drifting sidebar / 侧栏", &book);
        let store_on_error = store.clone();
        let task = runtime.spawn(async move {
            let connection = create.await.map_err(anyhow::Error::msg)?;
            let result = (|| -> anyhow::Result<_> {
                let cwd = dr_platform::AppDirs::for_identity(dr_platform::Identity::current())?
                    .data
                    .join("external")
                    .join(book)
                    .join(id);
                std::fs::create_dir_all(&cwd)?;
                let process = ExternalSession::spawn(&command)?;
                process.send(wire.initialize())?;
                Ok((process, connection.config, cwd))
            })();
            match result {
                Ok((process, config, cwd)) => {
                    Ok((process, config, cwd, connection.grant.id, remote))
                }
                Err(error) => {
                    let _ = store_on_error.revoke(&connection.grant.id).await;
                    Err(error)
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task
                .await
                .map_err(|_| "Agent launch worker stopped".to_owned())
                .and_then(|value| value.map_err(|error| error.to_string()));
            let mut result = Some(result);
            let updated = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.opening = false;
                match result.take().unwrap() {
                    Ok((process, config, cwd, grant, remote)) => {
                        this.pending_open = Some((config, cwd, remote));
                        this.process = Some(process);
                        this.grant = Some((store.clone(), grant));
                        this.status = "Initializing / 初始化".into();
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
            if (updated.is_err() || result.is_some())
                && let Some(Ok((_, _, _, id, _))) = result.take()
            {
                drop(store.revoke(&id));
            }
        })
        .detach();
        cx.notify();
    }
    fn send_rpc(&mut self, value: Value) -> bool {
        match self
            .process
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Connect the platform first / 请先连接平台"))
            .and_then(|process| process.send(value))
        {
            Ok(()) => true,
            Err(error) => {
                self.status = error.to_string();
                self.ready = false;
                self.running = false;
                false
            }
        }
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        let grant_was_revoked = self
            .grant
            .as_ref()
            .is_some_and(|(store, id)| !store.list().iter().any(|grant| &grant.id == id));
        if grant_was_revoked {
            self.disconnect(cx);
            self.status = "Tool access was revoked / 工具权限已撤销".into();
            return;
        }
        let mut changed = false;
        for _ in 0..128 {
            let event = self.process.as_mut().and_then(ExternalSession::try_recv);
            let Some(event) = event else { break };
            changed = true;
            match event {
                Err(error) => {
                    self.status = error;
                    self.process = None;
                    self.ready = false;
                    self.running = false;
                    break;
                }
                Ok(message) => self.message(message, cx),
            }
        }
        if changed {
            cx.notify();
        }
    }
    fn message(&mut self, message: Value, cx: &mut Context<Self>) {
        if let Some(error) = message.get("error") {
            self.status = format!(
                "External platform error / 平台错误: {}",
                error["message"].as_str().unwrap_or("request failed")
            );
            self.running = false;
            return;
        }
        if message["id"] == 1 && message.get("result").is_some() {
            if self.wire.platform == ExternalPlatform::Codex {
                self.send_rpc(json!({"jsonrpc":"2.0","method":"initialized"}));
            }
            if let Some((config, cwd, remote)) = self.pending_open.take() {
                let mcp = McpCommand {
                    command: config.command.to_string_lossy().into_owned(),
                    args: config.args,
                };
                match self
                    .wire
                    .open(&cwd.to_string_lossy(), remote.as_deref(), &mcp)
                {
                    Ok(value) => {
                        self.send_rpc(value);
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            return;
        }
        if message["id"] == 2 {
            if let Some(id) = self.wire.session_id(&message) {
                self.saved.remote = Some(id);
                self.ready = true;
                self.status = "Ready / 已连接".into();
                self.save(cx);
            }
            return;
        }
        if message.get("id").is_some() && message.get("method").is_some() {
            self.request = Some(message);
            self.status = "Platform requests input / 平台请求输入".into();
            return;
        }
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        if let Some(turn) = params.pointer("/turn/id").and_then(Value::as_str) {
            self.turn = Some(turn.to_owned());
        }
        if method == "item/agentMessage/delta" {
            self.append_delta(params["delta"].as_str().unwrap_or_default(), cx);
        } else if method == "session/update" {
            let update = &params["update"];
            match update["sessionUpdate"].as_str().unwrap_or_default() {
                "agent_message_chunk" => {
                    self.append_delta(update["content"]["text"].as_str().unwrap_or_default(), cx)
                }
                "tool_call" | "tool_call_update" => {
                    self.status = format!(
                        "Tool / 工具: {} · {}",
                        update["title"].as_str().unwrap_or(""),
                        update["status"].as_str().unwrap_or("running")
                    )
                }
                _ => {}
            }
        } else if method == "item/started" || method == "item/completed" {
            self.status = format!(
                "{} · {}",
                params["item"]["type"].as_str().unwrap_or("tool"),
                method
            );
        }
        if method == "turn/completed"
            || (self.wire.platform == ExternalPlatform::DeepseekHarness
                && message.get("result").is_some()
                && message["id"].as_u64().is_some_and(|id| id >= 10))
        {
            self.running = false;
            self.turn = None;
            self.status = "Ready / 已连接".into();
            self.save(cx);
        }
    }
    fn append_delta(&mut self, text: &str, cx: &mut App) {
        if let Some(last) = self.saved.messages.last_mut()
            && last.starts_with("Assistant / 助手: ")
            && last.len() < 512 * 1024
        {
            last.push_str(text);
            return;
        }
        self.remember(format!("Assistant / 助手: {text}"), cx);
    }
    fn send(&mut self, cx: &mut Context<Self>) {
        if !self.ready || self.running {
            return;
        }
        let text = self.input.read(cx).text();
        if text.trim().is_empty() {
            return;
        }
        let Some(session) = self.saved.remote.clone() else {
            return;
        };
        self.serial += 1;
        if self.send_rpc(self.wire.prompt(self.serial, &session, &text)) {
            self.remember(format!("You / 你: {text}"), cx);
            self.input.update(cx, |field, cx| field.set_text("", cx));
            self.running = true;
            self.status = "Running / 运行中".into();
        }
        cx.notify();
    }
    fn respond(&mut self, value: Value, cx: &mut Context<Self>) {
        if let Some(request) = self.request.take() {
            self.send_rpc(json!({"jsonrpc":"2.0","id":request["id"],"result":value}));
        }
        cx.notify();
    }
}
impl Render for ExternalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(window, cx).palette;
        let mut transcript = div().flex().flex_col().gap(px(8.));
        for (index, text) in self.saved.messages.iter().enumerate() {
            transcript = transcript.child(
                div()
                    .id(("external-message", index))
                    .child(dr_ui::text::wrapped(text.clone())),
            );
        }
        let request = self.request.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(10.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(
                        Button::new(
                            "external-connect",
                            if self.ready {
                                "Reconnect / 重连"
                            } else {
                                "Connect / 连接"
                            },
                        )
                        .small()
                        .disabled(self.opening || self.running)
                        .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                    )
                    .child(
                        Button::new("external-new", "New conversation / 新对话")
                            .small()
                            .disabled(self.running)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.disconnect(cx);
                                this.saved.id = uuid::Uuid::new_v4().to_string();
                                this.saved.remote = None;
                                this.saved.messages.clear();
                                this.save(cx);
                            })),
                    )
                    .child(
                        Button::new("external-disconnect", "Disconnect / 断开")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx))),
                    ),
            )
            .child(
                div()
                    .text_color(p.ink(3).solid())
                    .child(self.status.clone()),
            )
            .child(
                div()
                    .id("external-transcript")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(transcript),
            )
            .when_some(request, |el, request| {
                let mut options = div().flex().flex_wrap().gap(px(5.));
                if request["method"] == "session/request_permission" {
                    if let Some(choices) = request["params"]["options"].as_array() {
                        for choice in choices {
                            let id = choice["optionId"].clone();
                            let label = choice["name"].as_str().unwrap_or("Choose").to_owned();
                            options = options.child(
                                Button::new(format!("permission-{id}"), label)
                                    .small()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.respond(
                                            json!({"outcome":{"outcome":"selected","optionId":id}}),
                                            cx,
                                        )
                                    })),
                            );
                        }
                    }
                } else if request["method"]
                    .as_str()
                    .is_some_and(|method| method.ends_with("requestApproval"))
                {
                    for (name, decision) in [
                        ("Allow once / 允许一次", "accept"),
                        ("Decline / 拒绝", "decline"),
                    ] {
                        options = options.child(Button::new(decision, name).small().on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.respond(json!({"decision":decision}), cx)
                            }),
                        ));
                    }
                }
                el.child(dr_ui::text::wrapped(request["params"].to_string()))
                    .child(options)
                    .child(self.answer.clone())
                    .child(
                        Button::new("external-json-answer", "Reply JSON / 提交 JSON")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                match serde_json::from_str::<Value>(&this.answer.read(cx).text()) {
                                    Ok(value) => this.respond(value, cx),
                                    Err(_) => {
                                        this.status =
                                            "Enter a valid JSON response / 请输入有效 JSON".into();
                                        cx.notify();
                                    }
                                }
                            })),
                    )
            })
            .child(self.dictation.clone())
            .child(self.input.clone())
            .child(
                Button::new(
                    "external-send",
                    if self.running {
                        "Stop / 停止"
                    } else {
                        "Send / 发送"
                    },
                )
                .small()
                .primary()
                .disabled(!self.ready)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.running {
                        if let Some(session) = this.saved.remote.as_deref() {
                            match this
                                .wire
                                .cancel(this.serial + 1, session, this.turn.as_deref())
                            {
                                Ok(value) => {
                                    this.serial += 1;
                                    this.send_rpc(value);
                                }
                                Err(error) => {
                                    this.status = error.to_string();
                                }
                            }
                        }
                    } else {
                        this.send(cx);
                    }
                })),
            )
    }
}
