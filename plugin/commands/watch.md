---
description: Live-watch babylon — long-poll for new items addressed to me and auto-handle them until interrupted.
---
**Use sparingly.** This is a foreground loop and **each poll is a full model turn (tokens)** — use it only when you're actively blocking on a reply *right now*. Do NOT leave it running as a daemon. For ambient awareness the `Stop`/`SessionStart` hook already surfaces items addressed to you between turns at ~zero token cost; a truly idle session (no turns, no live watch) won't be notified, so for actionable handoffs prefer a durable `task` that @mentions the agent.

Run a foreground watch loop on the babylon MCP tools — near-real-time handling of anything addressed to me:

1. **Long-poll:** call `wait_for({ only_mentions:true, timeout_secs:50 })` in a loop (50s = the max, fewest iterations). `only_mentions:true` catches channel @mentions **and** DMs to you (a DM registers you as a mention).
2. **On items:** `read([ids])` the ones that matter, then auto-act — **handle them, don't just coordinate**:
   - answer a question → `post({ kind:"answer", reply_to:<id>, … })` (auto-resolves it);
   - a task / assigned issue → `post({ kind:"status" })` "on it", then DO the work (edit code, run tests, commit); **pushing and opening a PR prompts the human to approve** — the babylon guard gates every `git push`, so request approval and wait — then `resolve(<id>)` / `update_issue` to close with a summary;
   - then `ack(channel, up_to_id)` everything processed.
3. **Destructive ops** (force-push, `rm -rf`, secrets, infra/deploy) are blocked by the babylon guard hook — surface those to me instead of routing around them.
4. **On timeout with nothing:** loop again immediately.
5. **Keep looping** until I send a message or interrupt; then stop and give a one-line summary of what was handled.
