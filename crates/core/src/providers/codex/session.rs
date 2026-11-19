use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};

use crate::runtime::{CommandProcess, configure_account};
use crate::{Error, Provider, Result};

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct Session {
    process: CommandProcess,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    line: Vec<u8>,
    next_id: u64,
    completed: VecDeque<(String, bool)>,
}

impl Session {
    pub fn spawn(executable: &Path, home: &Path) -> Result<Self> {
        let mut command = Command::new(executable);
        command
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_account(&mut command, Provider::Codex, home);
        let mut process = CommandProcess::spawn(&mut command)?;
        let stdin = process.child.stdin.take().expect("piped stdin");
        let stdout = process.child.stdout.take().expect("piped stdout");
        Ok(Self {
            process,
            stdin,
            stdout: BufReader::new(stdout),
            line: Vec::new(),
            next_id: 1,
            completed: VecDeque::new(),
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "subscription-manager",
                    "title": "Subscription Manager",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {"experimentalApi": true},
            }),
        )
        .await?;
        tokio::time::timeout(
            RPC_TIMEOUT,
            self.send(json!({"jsonrpc": "2.0", "method": "initialized", "params": {}})),
        )
        .await
        .map_err(|_| Error::Timeout("Codex initialized".into()))??;
        Ok(())
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        tokio::time::timeout(RPC_TIMEOUT, self.request_inner(method, params))
            .await
            .map_err(|_| Error::Timeout(format!("Codex {method}")))?
    }

    async fn request_inner(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        loop {
            let envelope = self.read_envelope().await?;
            if envelope.get("method").is_some() {
                self.handle_message(envelope).await?;
                continue;
            }
            if envelope["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = envelope.get("error") {
                let code = error["code"].as_i64().unwrap_or_default();
                return Err(Error::Provider(format!(
                    "Codex {method} failed (RPC error {code})."
                )));
            }
            return envelope
                .get("result")
                .cloned()
                .ok_or_else(|| Error::Provider(format!("Codex {method} returned no result.")));
        }
    }

    pub async fn wait_for_login(&mut self, id: &str) -> Result<bool> {
        loop {
            if let Some(index) = self
                .completed
                .iter()
                .position(|(completed, _)| completed == id)
            {
                return Ok(self
                    .completed
                    .remove(index)
                    .expect("located login result")
                    .1);
            }
            let message = self.read_envelope().await?;
            self.handle_message(message).await?;
        }
    }

    pub async fn cancel_login(&mut self, id: &str) {
        // The server can close the local browser callback gracefully before its
        // subprocess group is stopped. Limit this even if the server is wedged.
        let _ = tokio::time::timeout(
            Duration::from_millis(750),
            self.request_inner("account/login/cancel", json!({"loginId": id})),
        )
        .await;
    }

    pub async fn stop(&mut self) {
        self.process.stop().await;
    }

    async fn handle_message(&mut self, envelope: Value) -> Result<()> {
        if envelope["method"].as_str() == Some("account/login/completed") {
            let params = &envelope["params"];
            if let Some(id) = params["loginId"].as_str() {
                if self.completed.len() == 16 {
                    self.completed.pop_front();
                }
                self.completed
                    .push_back((id.to_owned(), params["success"].as_bool().unwrap_or(false)));
            }
        } else if envelope.get("method").is_some()
            && let Some(id) = envelope
                .get("id")
                .filter(|id| id.is_number() || id.is_string())
        {
            self.send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "Method not supported by Subscription Manager"},
            })).await?;
        }
        Ok(())
    }

    async fn send(&mut self, envelope: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&envelope)
            .map_err(|_| Error::Provider("Could not encode Codex request.".into()))?;
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_envelope(&mut self) -> Result<Value> {
        loop {
            let available = self.stdout.fill_buf().await?;
            if available.is_empty() {
                return Err(Error::Provider(
                    "Codex app server stopped before it answered.".into(),
                ));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |index| index + 1);
            if self.line.len() + consumed > MAX_LINE_BYTES {
                return Err(Error::Provider(
                    "Codex app server emitted a message larger than 4 MiB.".into(),
                ));
            }
            self.line.extend_from_slice(&available[..consumed]);
            self.stdout.consume(consumed);
            if newline.is_some() {
                let envelope = serde_json::from_slice(&self.line);
                self.line.clear();
                if let Ok(value @ Value::Object(_)) = envelope {
                    return Ok(value);
                }
            }
        }
    }
}
