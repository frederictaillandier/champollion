//! Reads recordings on another machine over SSH: a Jetson running
//! `~/ocr/read-recording` (`gpu-ocr/jetson/`), which reads the recording
//! sent on its input with its GPU, and prints each frame's text, in order,
//! as Tesseract's TSV. The recording is streamed, not kept there.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Deserialize;

/// A frame's text, as the remote reader prints it.
#[derive(Deserialize)]
pub struct RemoteFrame {
    /// Time of the frame in the recording.
    pub seconds: f64,
    pub tsv: String,
}

pub enum Failure {
    /// The machine could not be reached: worth trying again later.
    Unreachable(String),
    /// It was reached but could not read the recording.
    Failed(String),
}

/// Lines of the error output kept for the log.
const ERROR_LINES: usize = 5;

/// Sends the recording at `path` to `host`, which reads `rate` frames per
/// second of it, and hands each frame's text to `on_frame`, in order.
/// Returns false when `stop` was set before the end.
pub fn read(
    host: &str,
    path: &Path,
    rate: u32,
    stop: &AtomicBool,
    on_frame: &mut dyn FnMut(RemoteFrame),
) -> Result<bool, Failure> {
    let file = File::open(path)
        .map_err(|e| Failure::Failed(format!("cannot open {}: {e}", path.display())))?;
    let mut child = Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
        .args(["-o", "ServerAliveInterval=15", host])
        .arg(format!("~/ocr/read-recording {rate}"))
        .stdin(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::Unreachable(format!("cannot run ssh: {e}")))?;

    let errors = Arc::new(Mutex::new(VecDeque::new()));
    let stderr = child.stderr.take().expect("piped stderr");
    let collector = {
        let errors = Arc::clone(&errors);
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if is_chatter(&line) {
                    continue;
                }
                let mut errors = errors.lock().unwrap();
                if errors.len() == ERROR_LINES {
                    errors.pop_front();
                }
                errors.push_back(line);
            }
        })
    };

    let stdout = child.stdout.take().expect("piped stdout");
    let mut finished = true;
    for line in BufReader::new(stdout).lines() {
        if stop.load(Ordering::Relaxed) {
            finished = false;
            let _ = child.kill();
            break;
        }
        let Ok(line) = line else { break };
        // NVIDIA's decoder library prints some chatter on the output too.
        if !line.starts_with('{') {
            continue;
        }
        match serde_json::from_str::<RemoteFrame>(&line) {
            Ok(frame) => on_frame(frame),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Failed(format!(
                    "unexpected output from {host}: {e}"
                )));
            }
        }
    }
    let status = child.wait().map_err(|e| Failure::Failed(e.to_string()))?;
    let _ = collector.join();
    if !finished || status.success() {
        return Ok(finished);
    }
    let errors = errors
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(" / ");
    let message = format!("{host}: {status}: {errors}");
    // ssh exits with 255 when it cannot connect.
    if status.code() == Some(255) {
        Err(Failure::Unreachable(message))
    } else {
        Err(Failure::Failed(message))
    }
}

/// What NVIDIA's libraries print each time a decoder opens.
fn is_chatter(line: &str) -> bool {
    ["NvMMLite", "Opening in", "NVMEDIA"]
        .iter()
        .any(|p| line.starts_with(p))
}
