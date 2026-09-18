# Clawde Plugin

A [Claude Code plugin](https://docs.anthropic.com/en/docs/claude-code/plugins) for the [Clawde](https://github.com/burakCokyildirim/clawde) macOS menu bar app. It reports real-time session state so the menu bar icon reflects what Claude Code is doing.

## How It Works

A background daemon tails the JSONL transcript and maintains a state machine that tracks Claude's activity. The daemon writes a `.cstatus` JSON file whenever state changes, and posts a Darwin notification so the menu bar app refreshes instantly.

### Session States

| State | Meaning |
|-------|---------|
| `active` | Claude is working — thinking, running tools, or has subagents running |
| `waiting` | Blocked on user input — permission prompt, question, or elicitation dialog |
| `idle` | Turn complete, nothing happening |
| `compacting` | Context compaction in progress |

### Architecture

```
SessionStart hook → spawns daemon → exits immediately
Daemon (background)
  → tails JSONL transcript every 100ms
  → tracks active subagents, detects questions, handles sticky compacting
  → writes .cstatus on state changes + posts Darwin notification
  → monitors Claude PID liveness → cleans up on exit

PermissionRequest / Notification hooks → write .csignal file → exit
SessionEnd hook → safety-net cleanup if daemon crashed
```

Only 4 hooks are registered (down from 12 in the Python version) because the daemon observes most state directly from the transcript.

### Slash Command

`/name-session <name>` — sets a custom display name for the session in the menu bar.

## Installation

Install via the Claude Code marketplace:

```bash
claude plugins install burakCokyildirim/clawde-plugin
```

Or add manually to your Claude Code plugin configuration.

## Requirements

- macOS (uses Darwin notifications and libproc FFI)
- [Clawde](https://github.com/burakCokyildirim/clawde) menu bar app

## License

BSD-3-Clause
