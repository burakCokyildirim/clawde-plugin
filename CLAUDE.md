# CLAUDE.md

## Project Overview

Rust implementation of the Clawde plugin hook scripts. This repo serves as both a Claude Code **marketplace** and a **plugin** for the [Clawde](https://github.com/burakCokyildirim/clawde) macOS menu bar app.

The plugin provides two Rust binaries:
- **session-status** — Daemon that tails JSONL transcripts, maintains a state machine, and writes `.cstatus` JSON files
- **set-session-name** — Sets a custom display name for a session by updating its `.cstatus` file

## Build & Test

```bash
cargo build --release
cargo test

# Single crate
cargo build --release -p session-status
cargo test -p session-status
```

## Deploy to Plugin Scripts

After building, copy binaries and re-sign them (required to clear `com.apple.provenance` xattr that causes Gatekeeper kills):

```bash
cp target/release/session-status plugins/clawde/scripts/session-status
cp target/release/set-session-name plugins/clawde/scripts/set-session-name
codesign -fs - plugins/clawde/scripts/session-status
codesign -fs - plugins/clawde/scripts/set-session-name
```

## Repository Layout

```
Cargo.toml                              # Workspace root
crates/
  session-status/                       # Daemon + hook binary
    Cargo.toml
    src/main.rs
  set-session-name/                     # Session naming utility
    Cargo.toml
    src/main.rs
  jsonl-analyzer/                       # JSONL transcript schema analyzer

.claude-plugin/
  marketplace.json                      # Marketplace definition

plugins/clawde/                  # Plugin distributed to users
  .claude-plugin/
    plugin.json                         # Plugin metadata
  hooks/
    hooks.json                          # 4 hook event registrations
  scripts/                              # Compiled binaries go here
    session-status                      # (built from crates/session-status)
    set-session-name                    # (built from crates/set-session-name)
  skills/
    session-name/SKILL.md               # /name-session slash command

jsonl-spec/                             # JSONL transcript schema docs
claude-json-specs/                      # Claude JSON schema docs
PRD.md                                  # Original product requirements (historical)
```

## Architecture

### session-status Binary Modes

Single binary, three modes selected by args:

| Mode | Trigger | Behavior |
|------|---------|----------|
| **Hook** | No flags (SessionStart/SessionEnd) | Reads stdin JSON. SessionStart spawns daemon + writes initial `.cstatus`. SessionEnd is safety-net cleanup. |
| **Daemon** | `--daemon` | Long-running background process. Tails JSONL, processes state machine, writes `.cstatus` on changes, monitors PID liveness. |
| **Signal** | `--signal` | Reads stdin JSON, writes `.csignal` file for daemon. Used by PermissionRequest and Notification hooks. |

### Hooks Registered (4)

| Hook | Mode | Purpose |
|------|------|---------|
| SessionStart | hook | Spawns daemon |
| PermissionRequest | signal | Tells daemon about permission prompts |
| Notification | signal | Tells daemon about elicitation/idle events |
| SessionEnd | hook | Safety-net cleanup if daemon crashed |

### JSONL State Machine

The daemon reads JSONL lines and derives state. Key patterns:

- `type:"assistant"` + `stop_reason:null` → `active` (streaming)
- `type:"assistant"` + `stop_reason:"tool_use"` → `active` with tool name
- `type:"assistant"` + `stop_reason:"end_turn"` → question detection or idle (unless agents active)
- `type:"user"` with text content → `active` / `"thinking"`
- `type:"user"` with `tool_result` → removes agent from tracking set
- `type:"user"` with the `tool_result` for the call a permission prompt is holding → ends `waiting` (the answer lets the call run, so its result is the first sign the user responded; see `awaiting_permission_for`)
- `type:"user"` whose text or `tool_result` is `[Request interrupted by user…]` → `idle` (a stopped turn; checked before the prompt rule, because the line carries a promptId and would otherwise read as a new prompt)
- `type:"progress"` → `active` with activity (subagent/bash/mcp)
- `agentId` starting with `"acompact-"` → `compacting` (any event type)
- `type:"system"` + `subtype:"compact_boundary"` → `idle` (compaction finished)

### Agent Tracking

`active_agents: HashSet<String>` keyed by `tool_use.id`. When `end_turn` fires but agents are still active, state stays `active`/`"subagent"` instead of going idle.

### Key Design Decisions

- **Daemon architecture** — tails JSONL continuously, maintains full state in memory, reacts in real-time
- **Atomic file writes** — temp file + rename in the same directory
- **Darwin notifications** via `notify_post()` FFI — no subprocess overhead
- **PID resolution** via `libproc` `proc_pidinfo` — no `ps` subprocess
- **PID liveness** via `kill(pid, 0)` — one syscall per 100ms poll cycle
- **Signal files** (`.csignal`) — hooks that can't be observed from JSONL (permission, elicitation) write signals for the daemon

### Communication Channels

| Channel | Direction | Purpose |
|---------|-----------|---------|
| stdin (JSON) | Claude Code → session-status | Hook event payload |
| argv | Claude Code → set-session-name | Session name argument |
| `.cstatus` file | binaries → Clawde app | Session state on disk |
| `.csignal` file | signal hooks → daemon | UI-only events not in JSONL |
| Darwin notification | binaries → Clawde app | Instant refresh signal |
| `CLAUDE_PID` env var | Claude Code → binaries | PID of the Claude Code process |

### Session States

| State | Description |
|-------|-------------|
| `active` | Claude is working (thinking, tool use, subagent) |
| `waiting` | Blocked on user input (permission, question, elicitation) |
| `idle` | Turn complete, no activity |
| `compacting` | Context compaction in progress |

### .cstatus File Format

```json
{"session_id":"<uuid>","pid":<int>,"ppid":<int>,"state":"<state>","activity":"<activity>","timestamp":"<ISO8601Z>","cwd":"<path>","event":"<last_event>"}
```

Optional `session_name` field preserved across writes if present.

## Releasing

Before creating a release, you **MUST** update the version in all three locations:

1. `plugins/clawde/.claude-plugin/plugin.json` — the `"version"` field
2. `.claude-plugin/marketplace.json` — the `"version"` field in the plugin entry
3. `Cargo.toml` workspace members (if the Cargo version is being bumped)

The `plugin.json` version is what Claude Code uses to determine the installed version. If it doesn't match the release tag, users will not get the correct version when installing via the marketplace.

## Platform

- **macOS only** (Darwin notifications, libproc FFI)
- Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`
