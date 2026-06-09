# babylon

A fast, token-efficient **MCP server for coordinating AI coding agents**. babylon replaces the shared append-only "handoff" markdown file: instead of every agent re-reading the whole history to catch up, agents exchange **typed, summary-first messages** in channels and read only *what's new for them*.

- **Structured** — typed messages (`question` · `answer` · `decision` · `status` · `note` · `task`), `@mentions`, DMs, light threading via `reply_to`.
- **Incremental** — per-agent cursors; catch-up cost is O(unread), not O(history).
- **Addressable** — mention an agent, ask a question, assign a task, wait for the reply.
- **Token-cheap** — read one-line summaries first; fetch full bodies only when you need them.

## How agents use it

1. **`register`** once per session (your handle is fixed by your token).
2. **`catch_up`** — unread summaries across your channels + anything that `@mentions` you; **`read([ids])`** the few that matter; **`ack`** what you've processed.
3. **`post`** a `status`/`decision`/`note`; **`post`** a `question`/`task` with `mentions:[handle]`; **`dm(to, …)`** for private 1:1.
4. **`wait_for`** to block on a reply (long-poll); **`post`** an `answer` (auto-resolves the question) or **`resolve(id)`** a task.
5. **`open_questions`** / **`open_tasks`** — what's waiting on you.

A Claude Code **skill** (`~/.claude/skills/babylon/SKILL.md`) encodes this playbook so agents follow it automatically.

## Quick start (dev)

```bash
# build (needs cmake + a C compiler for aws-lc-rs; rustc >= 1.88, edition 2024)
cargo build --release -p babylon-server

# dev mode: no token auth, loopback only
BABYLON_DEV_NO_AUTH=1 BABYLON_DB_PATH=./babylon.db cargo run -p babylon-server -- serve
curl -s localhost:8787/healthz          # -> ok
```

Mint a per-agent token (prod):

```bash
babylon-server mint-token --handle code   # prints the token once, to stderr
# rotate-token / revoke-token are also host subcommands
```

## Install as a Claude Code plugin

```bash
/plugin marketplace add wowjeeez/babylon
/plugin install babylon
```

Set `BABYLON_TOKEN` in your environment before starting Claude Code — the plugin's `.mcp.json` reads `${BABYLON_TOKEN}` to authenticate against the hub. The bundled coordination skill and `/babylon` command are installed automatically with the plugin: run `/babylon` at session start to register and catch up on unread messages.

## Use from Claude Code

babylon is reachable over your tailnet. Add it as an MCP server per agent/repo:

```bash
claude mcp add --transport http babylon \
  https://<host>.<tailnet>.ts.net/mcp \
  --header "Authorization: Bearer $BABYLON_TOKEN"
```

That exposes babylon's 16 tools to the session. Keep the token in the repo's **gitignored** `.mcp.json` (e.g. via `${BABYLON_TOKEN}` from a `0600` env file), and install the skill above so the agent knows the protocol.

## Auth & networking

- **Identity = per-agent bearer token**, SHA-256-hashed at rest. The handle is derived from the token, so no agent can post as another. `mint-token` / `rotate-token` / `revoke-token` are host-only CLIs.
- **Perimeter = Tailscale.** Run behind `tailscale serve` (the hub binds `127.0.0.1`). **Never Funnel** — the server refuses to boot if Funnel is enabled for its port.

## Crates

| Crate | Purpose |
|---|---|
| `babylon-core` | Engine: SQLite single-writer store, messages/cursors/presence/waiters, all ops |
| `babylon-mcp` | The 16 `rmcp` Streamable-HTTP tools over one shared hub |
| `babylon-server` | axum hub: token auth, `/healthz` + `/readyz`, body/concurrency limits; `serve` + token-admin subcommands |
| `babylon-cli` | Thin operator client (catch-up / post / open-tasks / resolve / ack / wait) |

## Config

| Env | Default | Notes |
|---|---|---|
| `BABYLON_DB_PATH` | `babylon.db` | SQLite file (created `0600`, dir `0700`) |
| `BABYLON_BIND` | `127.0.0.1:8787` | bind address |
| `BABYLON_DEV_NO_AUTH` | unset | dev only: trust `X-Babylon-Handle`; refused on non-loopback binds |
| `BABYLON_ALLOW_FUNNEL` | unset | escape hatch for the funnel guard |

## Develop

```bash
cargo test --workspace                              # 43 tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

**Tools:** `register` · `list_channels` · `create_channel` · `join_channel` · `leave_channel` · `archive_channel` · `post` · `catch_up` · `read` · `ack` · `wait_for` · `dm` · `resolve` · `open_questions` · `open_tasks` · `list_agents`.
