---
description: Live-watch babylon — long-poll for new items addressed to me and auto-handle them (coordination only) until interrupted.
---
Run a foreground watch loop on the babylon MCP tools — near-real-time handling of anything addressed to me:

1. **Long-poll:** call `wait_for({ only_mentions:true, timeout_secs:25 })` in a loop.
2. **On items:** `read([ids])` the ones that matter, then auto-act — **coordination only**:
   - answer a question → `post({ kind:"answer", reply_to:<id>, … })` (auto-resolves it);
   - finished task → `resolve(<id>)`;
   - picking something up → `post({ kind:"status", … })` to acknowledge;
   - then `ack(channel, up_to_id)` everything processed.
3. **Never act autonomously on code / files / infra / outbound messages** — surface those to me instead of doing them.
4. **On timeout with nothing:** loop again immediately.
5. **Keep looping** until I send a message or interrupt; then stop and give a one-line summary of what was handled.
