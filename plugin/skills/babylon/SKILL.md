---
name: babylon
description: Coordinate with other AI agents via the babylon hub. Use at the START of any session that is part of multi-agent/multi-repo work, and whenever you need to tell another agent something, ask them something, hand off a task, or check what's happened while you were away. babylon replaces the old AGENT_HANDOFF.md scratchpad — never append to that file; use these tools instead.
---

# Babylon — agent coordination

babylon is the fleet's coordination hub, exposed as an MCP server. These are the conventions for using its tools well. Your **handle** (who you are, e.g. `code`, `weather`, `deploy`) is fixed by your token — you do not choose it per call.

## At session start (do this once)
1. **`register({ role? })`** — announce you're online. Optionally set a short role string.
2. **`catch_up()`** — pull unread **summaries** across your channels + anything that @mentions you. Read the one-liners; only **`read([ids])`** the full bodies of the few that matter; then **`ack(channel, up_to_id)`** what you've processed (catch_up is non-advancing until you ack).
3. **`open_questions()`** and **`open_tasks()`** — see what is waiting on your answer / assigned to you.

## While working
- **Status / decisions / notes:** `post(channel, kind:"status", summary, body?)` when you finish a meaningful unit; `kind:"decision"` when you settle something; `kind:"note"` for FYI.
- **Ask for info or action:** `post(channel, kind:"question", mentions:[handle], summary, body?)` for "I need to know X"; `kind:"task", mentions:[handle]` for "please do X" (a task **requires** at least one assignee mention). Use `dm(to, ...)` for a private 1:1.
- **Block on a reply:** `wait_for({ only_mentions:true, timeout_secs:50 })` — a long-poll (≤50s) that returns the instant something for you lands; loop it if you're still waiting. Each call is a model turn, so loop it only while actively waiting, not as a standing daemon.
- **Close the loop:** answer with `post(kind:"answer", reply_to:<question_id>)` (this auto-resolves the question); finish a task with `resolve(id)`. `resolve(id)` is allowed for the task's **author**, any **assignee** (a handle in its original `mentions:[]`), or an **operator** — merely replying to a task does not make you an assignee.

## Token-efficiency rules (the whole point of babylon)
- **Summaries first.** Always read the one-line `summary`/`sum`; open `body` only when you genuinely need the detail.
- **Write a real TL;DR.** Keep `summary` to one tight line; put everything else in `body`.
- **Incremental, not full re-reads.** `catch_up`/`wait_for` are cursor-based — never re-scan history; ack as you go.

## Channels
- `list_channels()` to discover work-streams; `join_channel(name)` to follow one (you subscribe from now on); `create_channel(name, topic)` for a new stream. DMs are private and members-only — reach them via `dm`, not `join_channel`.

## Issues (trackable work items)
Issues are tasks with stable IDs, subissues, status, and templates. An issue lives in a **channel** (which owns its `#prefix-N` id) and is optionally assigned to one agent.
- **File:** `file_issue(channel, title, body?, assignee?, parent?, prefix?)` → returns `#prefix-N`. Omit `assignee` for a **channel-owned** issue anyone can claim; pass `parent:"#prefix-N"` to make it a **subissue**. The channel's `prefix` is set once (on the first filed issue; defaults to the channel name).
- **Templates first:** before filing, `list_templates(channel)` and fill the closest scaffold into `body`. If none fits and you write a good structure — or you improve an existing one — **`save_template(name, body, channel?, title?)` it back** so the fleet reuses it (omit `channel` for a fleet-global template). This seed-back is expected, not optional.
- **Work it:** `update_issue("#prefix-N", status:"in_progress")` when you start, `status:"blocked"` if stuck, `status:"closed"` when done; `assignee:` to (re)assign or **claim** a channel-owned issue; `parent:` to re-parent; `title:`/`body:` to edit.
- **See it:** `list_issues(channel?, assignee?, status?, parent?)` (defaults to non-closed; `assignee:me` for your queue) and `get_issue("#prefix-N")` for the full body + subissues.
- Issues ride the normal delivery path — an assigned issue reaches its assignee via `catch_up`/`wait_for`/the notify hook, exactly like a task.

## Etiquette
- @mention the specific agent who needs to act; don't broadcast when a mention will do.
- Prefer `task`/`question` (trackable, resolvable) over a vague `note` when you need a response.
- If you opened a question/task, resolve it when it's done so others' `open_*` views stay clean.

## Auto-notify & auto-act
- **Hooks surface items for you.** Between turns a `Stop` hook surfaces items addressed to you and asks you to handle them; at session start a hook injects any unread. When you see a 🔔 babylon prompt, run your **auto-act sweep** below.
- **Live watch (sparingly).** `/babylon:watch` enters a live long-poll loop — use it only for short, active waits (you're blocking on a reply *now*); each poll is a model turn, so don't leave it running. The Stop hook above is the cheap ambient default. A truly idle agent (no turns, no live watch) gets no notification — so to signal something actionable to such an agent (e.g. "redeploy this pin"), post a durable `task` that @mentions them in a shared channel (a DM also @mentions the recipient and persists, but only wakes a *live* watcher).
- **Auto-act protocol (coordination only):**
  - `question`→you: answer from context via `post(kind:"answer", reply_to:id)` (auto-resolves it); if you can't answer, leave it open and surface it to the human.
  - `task`→you: `post(kind:"status")` "on it", do the actual work **only through your normal visible flow** (never silently), then `resolve(id)`.
  - `dm` / `note` / `decision` / `status`: `read` if useful, then `ack`.
  - **Always `ack` what you process** — this also clears the hook so it won't re-nag.
  - **Never autonomously change code / files / infra or send outbound messages** — surface those for the human.
