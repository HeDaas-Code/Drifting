//! Small JSON-RPC-over-stdio bridge for external Agent platforms.
//!
//! The shell owns presentation; this module owns only process lifetime and
//! framed messages. Codex App Server and DeepSeek Harness ACP are selected by
//! an explicit command profile, so a failed external backend never falls back
//! to the in-process model.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalPlatform {
    Codex,
    DeepseekHarness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
            args: vec!["app-server".into() , "--listen-stdio".into()],
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

#[derive(Debug)]
pub struct ExternalSession {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    events: mpsc::Receiver<Result<Value, String>>,
}

impl ExternalSession {
    pub fn spawn(command: &ExternalCommand) -> Result<Self> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("could not start external Agent: {}", command.program))?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().context("external Agent stdin missing")?));
        let stdout = child.stdout.take().context("external Agent stdout missing")?;
        let (sender, events) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let result = line
                    .map_err(|error| error.to_string())
                    .and_then(|line| serde_json::from_str::<Value>(&line).map_err(|error| error.to_string()));
                if sender.send(result).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, stdin, events })
    }

    pub fn send(&self, message: &Value) -> Result<()> {
        let mut stdin = self.stdin.lock().map_err(|_| anyhow::anyhow!("external Agent stdin unavailable"))?;
        serde_json::to_writer(&mut *stdin, message)?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(())
    }

    pub fn try_recv(&self) -> Option<Result<Value, String>> {
        self.events.try_recv().ok()
    }

    pub fn initialize(&self, client_name: &str) -> Result<()> {
        self.send(&json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"clientInfo":{"name":client_name,"version":env!("CARGO_PKG_VERSION")},"protocolVersion":1}
        }))
    }

    pub fn start_session(&self, cwd: &str) -> Result<()> {
        if !std::path::Path::new(cwd).is_absolute() {
            bail!("external Agent session cwd must be absolute");
        }
        self.send(&json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"session/new",
            "params":{"cwd":cwd}
        }))
    }

    pub fn prompt(&self, session_id: &str, text: &str) -> Result<()> {
        self.send(&json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"session/prompt",
            "params":{"sessionId":session_id,"prompt":[{"type":"text","text":text}]}
        }))
    }

    pub fn cancel(&self, session_id: &str) -> Result<()> {
        self.send(&json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"session/cancel",
            "params":{"sessionId":session_id}
        }))
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
    fn command_profiles_are_explicit() {
        assert_eq!(ExternalCommand::codex().args, ["app-server", "--listen-stdio"]);
        assert_eq!(ExternalCommand::deepseek_harness().args, ["--profile", "acp"]);
    }

    #[test]
    fn cwd_must_be_absolute_before_writing_protocol() {
        let error = ExternalSession::spawn(&ExternalCommand {
            platform: ExternalPlatform::Codex,
            program: "definitely-not-installed".into(),
            args: Vec::new(),
        });
        assert!(error.is_err());
    }
}
