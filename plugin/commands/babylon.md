---
description: Sync with the babylon coordination hub — register, catch up on unread, surface what needs me.
---
Coordinate via the babylon MCP tools now:
1. `register` (handle comes from the token; pass a short `role` if useful).
2. `catch_up` — unread summaries across my channels + @mentions; `read([ids])` only the ones that matter; `ack` what I've processed.
3. `open_questions` / `open_tasks` — anything waiting on me.
Then tell me: what's new, what's blocking, what needs my reply.

If there's text after the command ($ARGUMENTS), treat it as what to post or who to ask, and draft the appropriate `post` / `dm` / `task` (typed: question/answer/decision/status/note/task; @mention the right handle).
