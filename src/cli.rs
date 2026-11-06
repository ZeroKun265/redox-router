//! Command-line control for a running RedoxRouter instance via a Unix domain socket.
//! Allows hotkeys, scripts, or Stream Deck buttons to switch setups or create virtual devices.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliRequest {
    ListSetups,
    ActivateSetup(String),
    NextSetup,
    PrevSetup,
    CreateSink { name: String, description: String },
    CreateMic { name: String, description: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedArgs {
    /// Start the GUI normally.
    RunGui,
    /// Print help text and exit.
    Help,
    /// Send a request to the running instance and exit.
    Request(CliRequest),
    /// Invalid CLI arguments.
    Error(String),
}

pub fn usage() -> &'static str {
    "RedoxRouter - PipeWire node patchbay\n\n\
     USAGE:\n  \
       redox-router                        Launch the GUI\n  \
       redox-router --list-setups | -l     List setups in the running instance\n  \
       redox-router --activate <NAME> | -a <NAME>\n                                      \
                                           Switch active setup to <NAME>\n  \
       redox-router --next-setup           Cycle to the next setup and make it active\n  \
       redox-router --prev-setup           Cycle to the previous setup and make it active\n  \
       redox-router --create-sink <NAME> [DESC]\n                                      \
                                           Create a virtual speaker sink on the active setup\n  \
       redox-router --create-mic <NAME> [DESC]\n                                      \
                                           Create a virtual microphone on the active setup\n  \
       redox-router --help | -h            Show this help message"
}

pub fn parse_args<I, S>(args: I) -> ParsedArgs
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let v: Vec<String> = args.into_iter().map(Into::into).collect();
    if v.is_empty() {
        return ParsedArgs::RunGui;
    }
    match v[0].as_str() {
        "-h" | "--help" => ParsedArgs::Help,
        "-l" | "--list-setups" => ParsedArgs::Request(CliRequest::ListSetups),
        "--next-setup" => ParsedArgs::Request(CliRequest::NextSetup),
        "--prev-setup" => ParsedArgs::Request(CliRequest::PrevSetup),
        "-a" | "--activate" => {
            if let Some(name) = v.get(1).filter(|s| !s.trim().is_empty()) {
                ParsedArgs::Request(CliRequest::ActivateSetup(name.clone()))
            } else {
                ParsedArgs::Error("missing setup name for --activate".to_string())
            }
        }
        "--create-sink" => {
            let Some(name) = v.get(1).filter(|s| !s.trim().is_empty()) else {
                return ParsedArgs::Error("missing technical name for --create-sink".to_string());
            };
            let description = v.get(2).cloned().unwrap_or_else(|| name.clone());
            ParsedArgs::Request(CliRequest::CreateSink {
                name: name.clone(),
                description,
            })
        }
        "--create-mic" => {
            let Some(name) = v.get(1).filter(|s| !s.trim().is_empty()) else {
                return ParsedArgs::Error("missing technical name for --create-mic".to_string());
            };
            let description = v.get(2).cloned().unwrap_or_else(|| name.clone());
            ParsedArgs::Request(CliRequest::CreateMic {
                name: name.clone(),
                description,
            })
        }
        other => ParsedArgs::Error(format!("unknown argument: {other}\n\n{}", usage())),
    }
}

pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join("redox-router.sock")
    } else {
        PathBuf::from("/tmp/redox-router.sock")
    }
}

/// Send a `CliRequest` to the running RedoxRouter instance and return its reply.
pub fn send_request(req: &CliRequest) -> Result<CliResponse, String> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!(
            "could not connect to running RedoxRouter at {}: {e}",
            path.display()
        )
    })?;
    let mut payload = serde_json::to_string(req).map_err(|e| e.to_string())?;
    payload.push('\n');
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| format!("invalid response: {e}"))
}

/// Incoming request waiting for the GUI thread to process and reply to.
pub struct IncomingCli {
    pub req: CliRequest,
    pub reply: Sender<CliResponse>,
}

/// Spawn the background Unix domain socket listener thread for the running GUI instance.
pub fn spawn_listener(ctx: eframe::egui::Context) -> Receiver<IncomingCli> {
    let (tx, rx) = mpsc::channel::<IncomingCli>();
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    if let Ok(listener) = UnixListener::bind(&path) {
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() {
                    if let Ok(req) = serde_json::from_str::<CliRequest>(&line) {
                        let (rtx, rrx) = mpsc::channel();
                        if tx.send(IncomingCli { req, reply: rtx }).is_ok() {
                            ctx.request_repaint();
                            if let Ok(resp) = rrx.recv_timeout(std::time::Duration::from_secs(3)) {
                                if let Ok(mut out) = serde_json::to_string(&resp) {
                                    out.push('\n');
                                    let mut writer = stream;
                                    let _ = writer.write_all(out.as_bytes());
                                    let _ = writer.flush();
                                }
                            }
                        }
                    }
                }
            }
        });
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_arg_parsing() {
        assert_eq!(parse_args(Vec::<String>::new()), ParsedArgs::RunGui);
        assert_eq!(parse_args(["--help"]), ParsedArgs::Help);
        assert_eq!(
            parse_args(["--list-setups"]),
            ParsedArgs::Request(CliRequest::ListSetups)
        );
        assert_eq!(
            parse_args(["--activate", "Streaming"]),
            ParsedArgs::Request(CliRequest::ActivateSetup("Streaming".to_string()))
        );
        assert_eq!(
            parse_args(["--next-setup"]),
            ParsedArgs::Request(CliRequest::NextSetup)
        );
        assert_eq!(
            parse_args(["--create-mic", "Voice_Mic", "Voice Chat Mic"]),
            ParsedArgs::Request(CliRequest::CreateMic {
                name: "Voice_Mic".to_string(),
                description: "Voice Chat Mic".to_string(),
            })
        );
        assert!(matches!(parse_args(["--activate"]), ParsedArgs::Error(_)));
    }
}

