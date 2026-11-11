//! Inherited JSON pipes to the existing browser adapter. UI never waits on I/O.
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
};
pub struct Backend {
    child: Child,
    input: Option<ChildStdin>,
    pub events: Receiver<Value>,
}
impl Backend {
    pub fn start(root: &Path) -> Result<Self, String> {
        let portable_node = root.join("runtime/node.exe");
        let script = if portable_node.is_file() {
            root.join("runtime/music-bridge.cjs")
        } else {
            root.join("native-dock/target/debug/music-bridge.cjs")
        };
        if !script.exists() {
            return Err(
                "Music adapter is missing. Run python run_native_dock.py --music to build it."
                    .into(),
            );
        }
        let mut child = Command::new(if portable_node.is_file() {
            portable_node.as_os_str()
        } else {
            std::ffi::OsStr::new("node")
        })
        .arg(script)
        .current_dir(root)
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("Cannot start Music adapter: {e}"))?;
        let input = child.stdin.take();
        let output = child.stdout.take().ok_or("Music adapter stdout missing")?;
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            // The owned adapter emits bounded metadata; no renderer objects cross threads.
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if line.len() > 1024 * 1024 {
                    continue;
                }
                if let Ok(value) = serde_json::from_str(&line) {
                    if tx.send(value).is_err() {
                        return;
                    }
                }
            }
            let _ = tx.send(serde_json::json!({"type":"error","error":"Music adapter disconnected. Reopen Music to reconnect."}));
        });
        Ok(Self {
            child,
            input,
            events,
        })
    }
    pub fn send(&mut self, value: Value) -> Result<(), String> {
        if let Some(input) = &mut self.input {
            writeln!(input, "{value}").map_err(|e| e.to_string())
        } else {
            Err("Music adapter is closed".into())
        }
    }
    pub fn exited(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.send(serde_json::json!({"action":"quit"}));
        self.input.take();
        // Windows releases the process handle on drop. The adapter closes its
        // own browser on EOF; never kill it before the browser pipe is closed.
    }
}
