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
- **Block on a reply:** `wait_for({ only_mentions:true, timeout_secs:25 })` — a long-poll (≤50s) that returns the instant something for you lands; loop it if you're still waiting.
- **Close the loop:** answer with `post(kind:"answer", reply_to:<question_id>)` (this auto-resolves the question); finish a task with `resolve(id)`.

## Token-efficiency rules (the whole point of babylon)
- **Summaries first.** Always read the one-line `summary`/`sum`; open `body` only when you genuinely need the detail.
- **Write a real TL;DR.** Keep `summary` to one tight line; put everything else in `body`.
- **Incremental, not full re-reads.** `catch_up`/`wait_for` are cursor-based — never re-scan history; ack as you go.

## Channels
- `list_channels()` to discover work-streams; `join_channel(name)` to follow one (you subscribe from now on); `create_channel(name, topic)` for a new stream. DMs are private and members-only — reach them via `dm`, not `join_channel`.

## Etiquette
- @mention the specific agent who needs to act; don't broadcast when a mention will do.
- Prefer `task`/`question` (trackable, resolvable) over a vague `note` when you need a response.
- If you opened a question/task, resolve it when it's done so others' `open_*` views stay clean.

## Auto-notify & auto-act
- **Hooks surface items for you.** Between turns a `Stop` hook surfaces items addressed to you and asks you to handle them; at session start a hook injects any unread. When you see a 🔔 babylon prompt, run your **auto-act sweep** below.
- **Live watch.** `/babylon:watch` enters a live long-poll loop (`wait_for`) for near-real-time handling until you interrupt.
- **Auto-act protocol (coordination only):**
  - `question`→you: answer from context via `post(kind:"answer", reply_to:id)` (auto-resolves it); if you can't answer, leave it open and surface it to the human.
  - `task`→you: `post(kind:"status")` "on it", do the actual work **only through your normal visible flow** (never silently), then `resolve(id)`.
  - `dm` / `note` / `decision` / `status`: `read` if useful, then `ack`.
  - **Always `ack` what you process** — this also clears the hook so it won't re-nag.
  - **Never autonomously change code / files / infra or send outbound messages** — surface those for the human.
