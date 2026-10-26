//! `mickey mcp` (PRD 0014): Mickey as a Model Context Protocol server, so
//! an AI agent (Claude Code: `claude mcp add mickey -- mickey mcp`) can see
//! and drive the Mickey on its own PC.
//!
//! MCP over stdio is JSON-RPC 2.0, one message per line: a request has an
//! `id` and wants one answer with the same `id`; a notification has none
//! and wants nothing. Mickey answers `initialize`, `ping`, `tools/list` and
//! `tools/call`. Each tool is a client of the running Mickey, as
//! `mickey-ui` is: it asks over the control channel and keeps no state.
//!
//! Never a tool that pairs a PC (a person must compare the two codes),
//! types, presses keys, reads the clipboard, or forgets a PC.

use crate::control::{self, Status};
use crate::identity;
use crate::trust;
use crate::update;
use mickey_config::{self as config, peers::Peers, version};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// The MCP revision this server speaks.
const PROTOCOL: &str = "2025-06-18";

/// One thing an agent may ask Mickey to do.
struct Tool {
    name: &'static str,
    description: &'static str,
    /// The JSON Schema of its arguments.
    input: fn() -> Value,
    /// What the agent reads back, or why it failed.
    run: fn(&Value) -> Result<String, String>,
}

fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "status",
            description: "Mickey on this PC: its version, which PC has the keyboard and mouse, every paired PC (online or not, and its Mickey version), who listens to every PC's sound, and whether a newer Mickey is out.",
            input: no_arguments,
            run: |_| status(),
        },
        Tool {
            name: "switch_keyboard",
            description: "Move the keyboard and mouse to a paired PC, by its name as `status` shows it, or back to this PC with \"this PC\".",
            input: || pc_argument("The PC to move the keyboard and mouse to, or \"this PC\"."),
            run: |arguments| {
                let pc = pc_named(arguments)?;
                let answer = if is_this_pc(&pc) {
                    ask("home")?
                } else {
                    ask(&format!("go {pc}"))?
                };
                answered(answer)
            },
        },
        Tool {
            name: "set_listener",
            description: "Choose the PC whose speakers or headphones play every PC's sound, by its name as `status` shows it, \"this PC\", or \"nobody\" (each PC plays its own). Every PC learns the choice.",
            input: || pc_argument("The PC that listens, \"this PC\", or \"nobody\"."),
            run: |arguments| {
                let pc = pc_named(arguments)?;
                let listener = if pc.eq_ignore_ascii_case("nobody") {
                    None
                } else {
                    Some(key_of(&pc)?)
                };
                change_audio(|audio| audio.listener = listener)?;
                Ok(format!("Listening to every PC's sound: {pc}."))
            },
        },
        Tool {
            name: "set_volume",
            description: "Set how loud one PC plays on the listener, 0 to 100, and whether it is muted. By its name as `status` shows it, or \"this PC\".",
            input: || {
                json!({
                    "type": "object",
                    "properties": {
                        "pc": { "type": "string", "description": "The PC whose sound to change." },
                        "volume": { "type": "integer", "minimum": 0, "maximum": 100 },
                        "muted": { "type": "boolean" },
                    },
                    "required": ["pc"],
                    "additionalProperties": false,
                })
            },
            run: |arguments| {
                let pc = pc_named(arguments)?;
                // The values first, so a wrong one is named as such
                // whatever the PC.
                let volume = match arguments.get("volume") {
                    None => None,
                    Some(volume) => Some(
                        volume
                            .as_u64()
                            .filter(|v| *v <= 100)
                            .ok_or("volume is 0 to 100")? as u8,
                    ),
                };
                let muted = arguments
                    .get("muted")
                    .map(|muted| muted.as_bool().ok_or("muted is true or false"))
                    .transpose()?;
                let key = key_of(&pc)?;
                change_audio(|audio| {
                    if let Some(volume) = volume {
                        audio.volumes.insert(key.clone(), volume);
                    }
                    match muted {
                        Some(true) if !audio.muted.contains(&key) => audio.muted.push(key.clone()),
                        Some(false) => audio.muted.retain(|muted| *muted != key),
                        _ => {}
                    }
                })?;
                Ok(format!("{pc}'s sound changed."))
            },
        },
        Tool {
            name: "check_update",
            description: "Ask GitHub now whether a newer Mickey is out than the one on this PC.",
            input: no_arguments,
            run: |_| {
                let latest = update::latest()?;
                Ok(if version::is_newer(&latest, version::THIS) {
                    format!(
                        "Mickey {latest} is out; this PC runs {}. `apply_update` installs it.",
                        version::THIS
                    )
                } else {
                    format!("Mickey {} is the latest.", version::THIS)
                })
            },
        },
        Tool {
            name: "apply_update",
            description: "Download the newer Mickey, check that the owner's release key signed it, and install it; Mickey restarts by itself. On Windows the user must confirm an administrator prompt. Ask the user before calling it.",
            input: no_arguments,
            run: |_| {
                let latest = update::latest()?;
                if !version::is_newer(&latest, version::THIS) {
                    return Ok(format!(
                        "Mickey {} is the latest: nothing to do.",
                        version::THIS
                    ));
                }
                update::apply(&latest)?;
                Ok(format!(
                    "Installing Mickey {latest}, checked against the release key; Mickey restarts by itself."
                ))
            },
        },
        Tool {
            name: "install",
            description: "Install Mickey on this PC from the latest release, checked against the owner's release key. On Windows it runs the installer (the user confirms an administrator prompt); on Linux it downloads and checks it, then gives the one command the user runs in a terminal (it asks for their password once).",
            input: no_arguments,
            run: |_| update::install_latest(),
        },
    ]
}

/// The schema of a tool that takes one PC.
fn pc_argument(description: &str) -> Value {
    json!({
        "type": "object",
        "properties": { "pc": { "type": "string", "description": description } },
        "required": ["pc"],
        "additionalProperties": false,
    })
}

fn pc_named(arguments: &Value) -> Result<String, String> {
    let pc = arguments
        .get("pc")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if pc.is_empty() {
        return Err("say which PC (`pc`), by its name as `status` shows it".into());
    }
    // The control channel reads one line: a name must not carry another.
    if pc.chars().any(char::is_control) {
        return Err("a PC's name has no control characters".into());
    }
    Ok(pc.to_string())
}

fn is_this_pc(pc: &str) -> bool {
    pc.eq_ignore_ascii_case("this PC") || pc.eq_ignore_ascii_case("this")
}

/// Mickey answers `error: …` when it could not.
fn answered(answer: String) -> Result<String, String> {
    match answer.strip_prefix("error: ") {
        Some(why) => Err(why.to_string()),
        None => Ok(answer),
    }
}

/// The public key (hex) of the PC called `pc` here, or of this PC.
fn key_of(pc: &str) -> Result<String, String> {
    if is_this_pc(pc) {
        let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
        return Ok(trust::to_hex(identity.public()));
    }
    let config = config::Config::load()?.ok_or("Mickey has no config.toml yet")?;
    let peer = config
        .peers
        .iter()
        .find(|peer| peer.name.eq_ignore_ascii_case(pc))
        .ok_or_else(|| format!("no paired PC is called {pc:?}; `status` lists them"))?;
    peer.key
        .clone()
        .ok_or_else(|| format!("{pc} has not connected yet, so its sound is not known"))
}

/// Changes `[audio]` in config.toml as a choice made now, and tells the
/// running Mickey, which uses it and shares it with every PC (as the
/// settings window's Save does).
fn change_audio(change: impl FnOnce(&mut config::Audio)) -> Result<(), String> {
    let path = config::path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut audio = config::parse(&text)?.audio;
    change(&mut audio);
    audio.since = config::now_seconds();
    config::write(&path, &config::set_audio(&text, &audio)?)?;
    answered(ask("arranged")?).map(|_| ())
}

fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

/// Answers on stdout, one line per request read on stdin, until stdin ends.
pub fn serve() -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|err| format!("cannot read stdin: {err}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(&line) {
            writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .map_err(|err| format!("cannot write stdout: {err}"))?;
        }
    }
    Ok(())
}

/// The reply to one message; `None` for a notification.
fn handle(line: &str) -> Option<Value> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Some(error(Value::Null, -32700, "not JSON"));
    };
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    Some(match answer(method, &params) {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, text)) => error(id, code, &text),
    })
}

fn error(id: Value, code: i64, text: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": text } })
}

fn answer(method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "mickey", "version": version::THIS },
            "instructions": "Mickey shares one keyboard, mouse, clipboard and sound between the user's PCs. Never pair a PC for the user: ask them to compare the six-digit codes themselves.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": tools().iter().map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": (tool.input)(),
            })).collect::<Vec<_>>(),
        })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == name)
                .ok_or((-32602, format!("no tool named {name:?}")))?;
            // A tool that fails is an answer the agent reads, not a protocol error.
            let (text, failed) = match (tool.run)(&arguments) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": failed }))
        }
        _ => Err((-32601, format!("no method {method:?}"))),
    }
}

/// The running Mickey's answer to `request`, all of it.
fn ask(request: &str) -> Result<String, String> {
    let lines: Result<Vec<String>, _> = control::request(request)
        .map_err(|_| NOT_RUNNING.to_string())?
        .collect();
    lines
        .map(|lines| lines.join("\n"))
        .map_err(|err| err.to_string())
}

const NOT_RUNNING: &str = "Mickey is not running on this PC. Start it: `systemctl --user start mickey` on Linux, or sign in again on Windows (it starts at login).";

fn status() -> Result<String, String> {
    let line = ask("status")?;
    let status = Status::from_line(&line).ok_or("Mickey's status was not readable")?;
    let peers = Peers::from_text(&ask("peers")?).unwrap_or_default();
    let config = config::Config::load().ok().flatten();
    let mut out = format!("Mickey {} on this PC", version::THIS);
    if let Some(config) = &config {
        out += &format!(" ({})", config.this_pc.name());
    }
    out += ".\n";
    let keyboard = match status.focus {
        None => "this PC".to_string(),
        Some(peer) => status
            .peers
            .get(peer)
            .map_or("another PC".into(), |(name, _)| name.clone()),
    };
    out += &format!("Keyboard and mouse: on {keyboard}.\n");
    if peers.peers.is_empty() {
        out += "No other PC is paired yet.\n";
    } else {
        out += "PCs:\n";
        for peer in &peers.peers {
            let state = if peer.online {
                "online"
            } else if peer.reinstalled {
                "refused: it answered with a new identity (reinstalled?)"
            } else {
                "offline"
            };
            let running = peer
                .version
                .as_deref()
                .map_or(String::new(), |v| format!(", Mickey {v}"));
            out += &format!("- {}: {state}{running}\n", peer.name);
        }
    }
    out += &format!(
        "Listening to every PC's sound: {}.\n",
        listener(config.as_ref())
    );
    // A Mickey from before PRD 0013 does not know `update`.
    let update = ask("update")?;
    out += &match version::checked(update.trim()) {
        Some(newer) => format!("Mickey {newer} is out: an update is waiting."),
        None if update.trim() == "none" => "Mickey is up to date.".to_string(),
        None => "Whether a newer Mickey is out: unknown (this Mickey is too old to say).".into(),
    };
    Ok(out)
}

/// Who listens, by the name the owner gave it here.
fn listener(config: Option<&config::Config>) -> String {
    let Some(key) = config.and_then(|config| config.audio.listener.clone()) else {
        return "nobody (each PC plays its own)".into();
    };
    let own = identity::data_dir()
        .and_then(|dir| identity::Identity::load_or_create(&dir))
        .map(|identity| trust::to_hex(identity.public()));
    if own.as_deref() == Ok(key.as_str()) {
        return "this PC".into();
    }
    config
        .and_then(|config| {
            config
                .peers
                .iter()
                .find(|peer| peer.key.as_deref() == Some(key.as_str()))
        })
        .map_or("a PC not paired here".into(), |peer| peer.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(line: &str) -> Value {
        handle(line).expect("a request is answered")
    }

    #[test]
    fn it_introduces_itself_and_lists_its_tools() {
        let hello = reply(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
        assert_eq!(hello["id"], 1);
        assert_eq!(hello["result"]["protocolVersion"], PROTOCOL);
        assert_eq!(hello["result"]["serverInfo"]["name"], "mickey");
        let list = reply(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#);
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"status"), "{names:?}");
        assert!(
            !names
                .iter()
                .any(|name| name.contains("pair") || name.contains("forget"))
        );
        for wanted in [
            "switch_keyboard",
            "set_listener",
            "set_volume",
            "check_update",
            "apply_update",
            "install",
        ] {
            assert!(names.contains(&wanted), "{wanted} missing from {names:?}");
        }
    }

    #[test]
    fn a_tool_without_its_pc_says_what_is_missing() {
        let call = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"switch_keyboard","arguments":{}}}"#;
        let answer = reply(call);
        assert_eq!(answer["result"]["isError"], true);
        assert!(
            answer["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("which PC")
        );
        assert_eq!(
            answered("error: no PC named X".into()),
            Err("no PC named X".into())
        );
    }

    #[test]
    fn a_volume_out_of_range_is_named_before_the_pc_is_looked_up() {
        let call = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"set_volume","arguments":{"pc":"No Such PC","volume":500}}}"#;
        let answer = reply(call);
        assert_eq!(answer["result"]["isError"], true);
        assert_eq!(answer["result"]["content"][0]["text"], "volume is 0 to 100");
    }

    #[test]
    fn a_name_cannot_carry_a_second_control_request() {
        let call = r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"switch_keyboard","arguments":{"pc":"Laptop\nquit"}}}"#;
        let answer = reply(call);
        assert_eq!(answer["result"]["isError"], true);
        assert!(
            answer["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("control")
        );
    }

    #[test]
    fn notifications_get_no_answer_and_mistakes_get_errors() {
        assert_eq!(
            handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
        );
        assert_eq!(reply("not json")["error"]["code"], -32700);
        assert_eq!(
            reply(r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#)["error"]["code"],
            -32601
        );
        let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"pair","arguments":{}}}"#;
        assert_eq!(reply(call)["error"]["code"], -32602);
    }
}
