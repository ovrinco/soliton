//! Local model availability is measured separately from a successful generation.
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
const MODEL: &str = "qwen3.8-27b-desktop";
fn manager() -> PathBuf {
    std::env::var_os("SOLITON_LLM_MANAGER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(""))
}
fn get(url: &str) -> Option<Value> {
    let output = Command::new("curl")
        .args([
            "--silent",
            "--fail",
            "--noproxy",
            "*",
            "--connect-timeout",
            "1",
            "--max-time",
            "2",
            "--max-filesize",
            "262144",
            url,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 262144 {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}
fn status() -> Value {
    let models = get("http://127.0.0.1:8080/v1/models");
    let advertised = models
        .as_ref()
        .and_then(|m| m["data"].as_array())
        .is_some_and(|items| items.iter().any(|m| m["id"] == MODEL));
    let ready = advertised
        && get("http://127.0.0.1:8092/health").is_some_and(|value| value["status"] == "ok");
    json!({"state":if ready{"ready"}else if advertised{"cold"}else if models.is_some(){"model_missing"}else{"offline"},"model":MODEL,"can_start":manager().is_file(),"local_only":true,"notice":if ready{"Local model is ready."}else if advertised{"Local model is available; the first request may take several minutes to load it."}else if models.is_some(){"The local gateway does not advertise the configured desktop model."}else{"Local model gateway is stopped or unavailable."}})
}
#[derive(Default)]
pub struct InferenceStatus {
    pending: Option<Receiver<Value>>,
    last: Option<Value>,
}
impl InferenceStatus {
    pub fn command(&mut self, command: &str) -> Result<Value> {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(value) => {
                    self.last = Some(value);
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.last = Some(
                        json!({"state":"failed","notice":"Local service check stopped. Try again."}),
                    );
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if command == "inference-start" {
            if self.pending.is_some() {
                return Ok(json!({"state":"checking","pending":true}));
            }
            let path = manager();
            if !path.is_absolute() || !path.is_file() {
                return Err(Error::Invalid(
                    "No local model manager is configured on this computer.".into(),
                ));
            }
            let (sender, receiver) = mpsc::channel();
            self.pending = Some(receiver);
            thread::spawn(move || {
                let result = (|| -> Result<Value> {
                    // Start the existing gateway only. Its configured model lifecycle remains authoritative.
                    let mut child = Command::new(path)
                        .args(["gateway", "start"])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()?;
                    let deadline = Instant::now() + Duration::from_secs(15);
                    loop {
                        if let Some(exit) = child.try_wait()? {
                            if !exit.success() {
                                return Err(Error::Invalid(format!(
                                    "Local gateway manager exited with {exit}"
                                )));
                            }
                            return Ok(status());
                        }
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(Error::Invalid(
                                "Local gateway startup timed out; retry its health check.".into(),
                            ));
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                })();
                let _ =
                    sender.send(result.unwrap_or_else(
                        |error| json!({"state":"failed","notice":error.to_string()}),
                    ));
            });
            self.last = Some(
                json!({"state":"starting","notice":"Starting the existing local model gateway…"}),
            );
        } else if command != "inference-status" && command != "inference-refresh" {
            return Err(Error::Invalid("Unknown local inference command".into()));
        } else if (self.last.is_none() || command == "inference-refresh") && self.pending.is_none()
        {
            let (sender, receiver) = mpsc::channel();
            self.pending = Some(receiver);
            thread::spawn(move || {
                let _ = sender.send(status());
            });
        }
        let mut value = self
            .last
            .clone()
            .unwrap_or_else(|| json!({"state":"checking","notice":"Checking the local model…"}));
        value["pending"] = json!(self.pending.is_some());
        Ok(value)
    }
}
