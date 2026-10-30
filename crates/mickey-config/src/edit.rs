//! Changes to config.toml made by Mickey itself, on the text: every other
//! line, comments included, stays as the owner wrote it, and the result
//! is checked with `parse` before it is returned.

use crate::{Audio, Hotkeys, parse};
use std::net::SocketAddr;

/// `text` (a config.toml) with the peer named `name` placed at `position`:
/// its `position` line replaced, or added under its `name`. Every other
/// line, comments included, is kept as it was. The result is checked with
/// the same rules as `parse`.
pub fn set_position(text: &str, name: &str, position: [i32; 2]) -> Result<String, String> {
    let wanted = format!("position = [{}, {}]", position[0], position[1]);
    set_peer_field(text, name, "position", wanted)
}

/// `text` with the peer named `name` bound to the public key `key` (64 hex
/// digits): Mickey then talks to that PC only with that key (#184).
pub fn set_key(text: &str, name: &str, key: &str) -> Result<String, String> {
    set_peer_field(text, name, "key", format!("key = {}", toml_string(key)))
}

/// `text` with the `field` line of the peer named `name` replaced by
/// `wanted`, or `wanted` added under its `name`. Every other line,
/// comments included, is kept; the result is checked like `parse`.
fn set_peer_field(text: &str, name: &str, field: &str, wanted: String) -> Result<String, String> {
    let is_field = |line: &str| {
        line.strip_prefix(field)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let is_our_name = |line: &str| {
        line.starts_with("name")
            && toml::from_str::<toml::Table>(line)
                .ok()
                .and_then(|table| table.get("name")?.as_str().map(|value| value == name))
                == Some(true)
    };
    let mut out: Vec<String> = Vec::new();
    let mut in_peer = false;
    // Where our peer's name line is in `out`, while inside its block.
    let mut ours: Option<usize> = None;
    let mut done = false;
    for line in text.lines().chain(std::iter::once("[end]")) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            // Leaving our block without a position line: add one under its name.
            if let Some(at) = ours.take()
                && !done
            {
                out.insert(at + 1, wanted.clone());
                done = true;
            }
            in_peer = trimmed.starts_with("[[peer]]");
        }
        if in_peer && !done && is_our_name(trimmed) {
            ours = Some(out.len());
        }
        if ours.is_some() && !done && is_field(trimmed) {
            out.push(wanted.clone());
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    out.pop(); // the "[end]" sentinel
    if !done {
        return Err(format!("no PC named {name:?} in config.toml"));
    }
    let mut text = out.join("\n");
    text.push('\n');
    parse(&text)?;
    Ok(text)
}

/// `text` (a config.toml) with its `[hotkeys]` section replaced by
/// `hotkeys`, in place, or added at the end. Other lines are kept, and the
/// result is checked with `parse`.
pub fn set_hotkeys(text: &str, hotkeys: &Hotkeys) -> Result<String, String> {
    set_section(text, "hotkeys", &hotkeys.to_lines())
}

/// `text` (a config.toml) with its `[audio]` section replaced by `audio`.
pub fn set_audio(text: &str, audio: &Audio) -> Result<String, String> {
    set_section(text, "audio", &audio.to_lines())
}

/// `text` with section `[name]` replaced by `lines`, in place, or added at
/// the end. Other lines are kept, and the result is checked with `parse`.
fn set_section(text: &str, name: &str, lines: &str) -> Result<String, String> {
    let header = format!("[{name}]");
    let section = format!("{header}\n{lines}");
    let mut out: Vec<String> = Vec::new();
    let (mut skipping, mut done) = (false, false);
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            skipping = trimmed.starts_with(&header);
            if skipping {
                out.push(section.clone());
                out.push(String::new());
                done = true;
            }
        }
        if !skipping {
            out.push(line.to_string());
        }
    }
    if !done {
        out.push(String::new());
        out.push(section);
    }
    let mut text = out.join("\n").trim_end().to_string();
    text.push('\n');
    parse(&text)?;
    Ok(text)
}

/// `text` (a config.toml) with a new peer at its end. A name already
/// taken gets " (2)", " (3)"… so two PCs never share one. Checked with
/// `parse`; returns the text and the name used.
pub fn add_peer(
    text: &str,
    name: &str,
    address: SocketAddr,
    position: Option<[i32; 2]>,
) -> Result<(String, String), String> {
    let taken = parse(text)?
        .peers
        .into_iter()
        .map(|peer| peer.name)
        .collect::<Vec<_>>();
    let name = (1..)
        .map(|n| {
            if n == 1 {
                name.to_string()
            } else {
                format!("{name} ({n})")
            }
        })
        .find(|candidate| !taken.contains(candidate))
        .expect("an unused name exists");
    let mut block = format!(
        "\n[[peer]]\nname = {}\naddress = \"{address}\"\n",
        toml_string(&name)
    );
    if let Some([x, y]) = position {
        block += &format!("position = [{x}, {y}]\n");
    }
    let mut text = text.trim_end().to_string();
    text += "\n";
    text += &block;
    parse(&text)?;
    Ok((text, name))
}

/// A TOML basic string: quotes and backslashes escaped.
pub fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}
