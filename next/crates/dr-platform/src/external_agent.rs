//! Explicit Codex App Server and DeepSeek Harness ACP stdio transports.
//! This module never chooses a model or reads credentials.
use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalPlatform {
    Codex,
    DeepseekHarness,
}
#[derive(Clone, Debug)]
pub struct ExternalCommand {
    pub platform: ExternalPlatform,
    pub program: String,
    pub args: Vec<String>,
}
impl ExternalCommand {
    pub fn codex() -> Self {
        Self {
            platform: ExternalPlatform::Codex,
            program: "codex".into(),
            args: vec!["app-server".into(), "--stdio".into()],
        }
    }
    pub fn deepseek_harness() -> Self {
        Self {
            platform: ExternalPlatform::DeepseekHarness,
            program: "dsh".into(),
            args: vec!["--profile".into(), "acp".into()],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCommand {
    pub command: String,
    pub args: Vec<String>,
}

/// RPC builders are platform-specific; ACP methods must never be sent to Codex.
#[derive(Clone, Debug)]
pub struct Wire {
    pub platform: ExternalPlatform,
}
impl Wire {
    fn request(&self, id: u64, method: &str, params: Value) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    }
    pub fn initialize(&self) -> Value {
        self.request(1, "initialize", match self.platform {
            ExternalPlatform::Codex => json!({"clientInfo":{"name":"drifting-next","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}}),
            ExternalPlatform::DeepseekHarness => json!({"protocolVersion":1,"clientInfo":{"name":"drifting-next","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{}}),
        })
    }
    pub fn open(&self, cwd: &str, resume: Option<&str>, mcp: &McpCommand) -> Result<Value> {
        if !std::path::Path::new(cwd).is_absolute() {
            bail!("External session directory must be absolute");
        }
        Ok(match self.platform {
            ExternalPlatform::Codex => {
                let config =
                    json!({"mcp_servers":{"drifting":{"command":mcp.command,"args":mcp.args}}});
                let mut params = json!({"cwd":cwd,"config":config});
                if let Some(id) = resume {
                    params["threadId"] = id.into();
                }
                self.request(
                    2,
                    if resume.is_some() {
                        "thread/resume"
                    } else {
                        "thread/start"
                    },
                    params,
                )
            }
            ExternalPlatform::DeepseekHarness => {
                let mut params = json!({"cwd":cwd,"mcpServers":[{"name":"drifting","command":mcp.command,"args":mcp.args,"env":[]}]});
                if let Some(id) = resume {
                    params["sessionId"] = id.into();
                }
                self.request(
                    2,
                    if resume.is_some() {
                        "session/load"
                    } else {
                        "session/new"
                    },
                    params,
                )
            }
        })
    }
    pub fn prompt(&self, id: u64, session: &str, text: &str) -> Value {
        match self.platform {
            ExternalPlatform::Codex => self.request(id, "turn/start", json!({"threadId":session,"input":[{"type":"text","text":text,"text_elements":[]}]})),
            ExternalPlatform::DeepseekHarness => self.request(id, "session/prompt", json!({"sessionId":session,"prompt":[{"type":"text","text":text}]})),
        }
    }
    pub fn cancel(&self, id: u64, session: &str, turn: Option<&str>) -> Result<Value> {
        Ok(match self.platform {
            ExternalPlatform::Codex => self.request(
                id,
                "turn/interrupt",
                json!({"threadId":session,"turnId":turn.context("Turn has not started yet")?}),
            ),
            ExternalPlatform::DeepseekHarness => {
                json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}})
            }
        })
    }
    /// Only the active prompt can finish an ACP turn. Codex completion status
    /// distinguishes success from interruption and failure.
    pub fn completion(&self, message: &Value, prompt: Option<u64>) -> Option<Result<(), String>> {
        match self.platform {
            ExternalPlatform::Codex if message["method"] == "turn/completed" => {
                let status = message
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                Some(if status == "completed" {
                    Ok(())
                } else {
                    Err(status.to_owned())
                })
            }
            ExternalPlatform::DeepseekHarness
                if prompt.is_some()
                    && message["id"].as_u64() == prompt
                    && message.get("result").is_some() =>
            {
                let reason = message["result"]["stopReason"]
                    .as_str()
                    .unwrap_or("unknown");
                Some(if reason == "end_turn" {
                    Ok(())
                } else {
                    Err(reason.to_owned())
                })
            }
            _ => None,
        }
    }
    pub fn session_id(&self, response: &Value) -> Option<String> {
        match self.platform {
            ExternalPlatform::Codex => response.pointer("/result/thread/id"),
            ExternalPlatform::DeepseekHarness => response.pointer("/result/sessionId"),
        }
        .and_then(Value::as_str)
        .map(str::to_owned)
    }
}

pub struct ExternalSession {
    child: Child,
    outgoing: SyncSender<Value>,
    events: Receiver<Result<Value, String>>,
    disconnected: bool,
}
impl ExternalSession {
    pub fn spawn(command: &ExternalCommand) -> Result<Self> {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd.spawn().context(
            "External Agent could not start; check the executable and its own configuration",
        )?;
        let mut stdin = child.stdin.take().context("Missing Agent stdin")?;
        let stdout = child.stdout.take().context("Missing Agent stdout")?;
        let (outgoing, messages) = mpsc::sync_channel::<Value>(32);
        let (sender, events) = mpsc::sync_channel(128);
        let errors = sender.clone();
        std::thread::spawn(move || {
            for message in messages {
                let result = (|| -> Result<()> {
                    serde_json::to_writer(&mut stdin, &message)?;
                    stdin.write_all(b"\n")?;
                    stdin.flush()?;
                    Ok(())
                })();
                if result.is_err() {
                    let _ = errors.send(Err("External Agent disconnected while sending".into()));
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                // Avoid an unbounded protocol line or log stream allocation.
                let mut bytes = Vec::new();
                let mut bounded = (&mut reader).take(4 * 1024 * 1024 + 1);
                use std::io::Read as _;
                match bounded.read_until(b'\n', &mut bytes) {
                    Ok(0) => {
                        let _ = sender.send(Err("External Agent disconnected".into()));
                        break;
                    }
                    Ok(_) if bytes.len() > 4 * 1024 * 1024 => {
                        let _ = sender.send(Err("External Agent message exceeds 4 MiB".into()));
                        break;
                    }
                    Ok(_) => {
                        if sender
                            .send(
                                serde_json::from_slice(&bytes)
                                    .map_err(|_| "Invalid external Agent message".into()),
                            )
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(Err("External Agent transport failed".into()));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            outgoing,
            events,
            disconnected: false,
        })
    }
    pub fn send(&self, message: Value) -> Result<()> {
        if message.to_string().len() > 512 * 1024 {
            bail!("External Agent input exceeds 512 KiB");
        }
        self.outgoing
            .try_send(message)
            .map_err(|_| anyhow::anyhow!("External Agent is not accepting messages"))
    }
    pub fn try_recv(&mut self) -> Option<Result<Value, String>> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) if !self.disconnected => {
                self.disconnected = true;
                Some(Err("External Agent disconnected".into()))
            }
            Err(_) => None,
        }
    }
}
impl Drop for ExternalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platforms_use_distinct_session_prompt_and_cancel_methods() {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().to_str().unwrap();
        let mcp = McpCommand {
            command: "/synthetic/drifting".into(),
            args: vec!["--mcp".into()],
        };
        for (platform, open, prompt, stop) in [
            (
                ExternalPlatform::Codex,
                "thread/start",
                "turn/start",
                "turn/interrupt",
            ),
            (
                ExternalPlatform::DeepseekHarness,
                "session/new",
                "session/prompt",
                "session/cancel",
            ),
        ] {
            let wire = Wire { platform };
            assert_eq!(wire.open(cwd, None, &mcp).unwrap()["method"], open);
            assert_eq!(wire.prompt(4, "s", "hello")["method"], prompt);
            assert_eq!(wire.cancel(5, "s", Some("turn")).unwrap()["method"], stop);
            assert!(wire.open("relative", None, &mcp).is_err());
        }
    }
    #[test]
    fn resume_and_mcp_configuration_are_not_global_config_edits() {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().to_str().unwrap();
        let wire = Wire {
            platform: ExternalPlatform::Codex,
        };
        let mcp = McpCommand {
            command: "/synthetic/drifting".into(),
            args: vec!["--mcp".into()],
        };
        let request = wire.open(cwd, Some("old-thread"), &mcp).unwrap();
        assert_eq!(request["method"], "thread/resume");
        assert_eq!(request["params"]["threadId"], "old-thread");
        assert_eq!(
            request["params"]["config"]["mcp_servers"]["drifting"]["command"],
            "/synthetic/drifting"
        );
        let acp = Wire {
            platform: ExternalPlatform::DeepseekHarness,
        };
        assert_eq!(
            acp.open(cwd, Some("s"), &mcp).unwrap()["method"],
            "session/load"
        );
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    #[test]
    fn terminal_status_and_the_active_acp_request_are_preserved() {
        let codex = Wire {
            platform: ExternalPlatform::Codex,
        };
        for status in ["completed", "failed", "interrupted"] {
            let result = codex
                .completion(
                    &json!({"method":"turn/completed", "params":{"turn":{"status":status}}}),
                    Some(11),
                )
                .unwrap();
            assert_eq!(result.is_ok(), status == "completed");
        }
        let acp = Wire {
            platform: ExternalPlatform::DeepseekHarness,
        };
        assert!(
            acp.completion(&json!({"id":12,"result":{}}), Some(11))
                .is_none()
        );
        assert!(
            acp.completion(&json!({"id":11,"result":{}}), None)
                .is_none()
        );
        assert!(
            acp.completion(
                &json!({"id":11,"result":{"stopReason":"end_turn"}}),
                Some(11)
            )
            .unwrap()
            .is_ok()
        );
    }
}
