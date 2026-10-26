# 0014 — An MCP server

> Status: **draft** · Depends on: [0013](0013-updates.md) (the update tools) · Vision: [0000](0000-vision.md)

## Goals

Let an AI agent (Claude Code, Claude Desktop, any MCP client) install,
update and drive Mickey on the PC it runs on, through the Model Context
Protocol. Decided with the owner on 2026-10-26: the agent may **see the
state, take safe actions, install and update**; it may never pair a PC.

- `claude mcp add mickey -- mickey mcp` is the whole setup.
- An agent can answer "which PC has my keyboard?", "is the laptop
  online?", "who is listening?", "is every PC up to date?".
- An agent can move the keyboard to a PC or home, pick the listener,
  set a PC's volume or mute it, check for an update and apply it, and
  install Mickey from the files it came with.

## Approach

- **`mickey mcp`** speaks MCP over stdio: JSON-RPC 2.0, one message per
  line, protocol version `2025-06-18`. It answers `initialize`,
  `tools/list` and `tools/call`, and ignores notifications.
- **It is a client of the running Mickey**, like `mickey-ui`: it asks
  over the control channel (`status`, `peers`, `go <pc>`, `home`) and
  changes `[audio]` through `mickey-config`, which the running Mickey
  already follows. It holds no state of its own.
- **Tools** (each with a JSON Schema for its input):

  | Tool | Does |
  |---|---|
  | `status` | Version, this PC, which PC has the keyboard, every PC with its state and version, the listener |
  | `switch_keyboard` | Move the keyboard and mouse to a PC by name, or `home` |
  | `set_listener` | The PC whose headphones play every PC's sound, or nobody |
  | `set_volume` | One PC's volume (0–100) and mute, on the listener |
  | `check_update` | Ask GitHub for a newer release (PRD 0013) |
  | `apply_update` | Download, verify and install it (PRD 0013) |
  | `install` | Run the installer that came with this `mickey` (a fresh PC) |

- **`serde_json`** parses and writes the messages. Hand-written JSON
  parsing is where bugs hide; `serde` is already in the tree for TOML.

## Requirements

- **No pairing tool, ever.** The six-digit code exists so a person
  compares two screens; an agent approving it would remove that check.
  `status` says when a PC asks to pair, so the agent can tell the owner.
- **No input, no clipboard.** No tool types, presses keys, or reads the
  clipboard or its history (hard rule 7).
- **No forgetting PCs.** Removing trust stays with the owner.
- Every answer an agent sees is the same text a person would see in
  `mickey status`; nothing secret (keys are shown as the usual short
  fingerprint).
- A tool that needs the running Mickey and finds none says so and how to
  start it.

## Non-Goals

- Driving another PC's Mickey (an agent talks to the Mickey on its own
  PC; that one already reaches the others).
- An HTTP transport. Stdio only, so nothing listens on the network.

## Discoveries
