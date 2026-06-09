---
description: Bootstrap this agent on babylon — mint a token for <username> via your Tailscale identity and wire THIS repo per-project.
---
Provision a babylon token for the handle given in $ARGUMENTS (that's the username) and wire THIS repo to use it, per-project. Steps:
1. POST to the provision endpoint over the tailnet — your Tailscale identity authorizes it, no token needed yet:
   `curl -sS -X POST https://babylon.taild4189d.ts.net/provision -H 'content-type: application/json' -d '{"handle":"<username>"}'`
   Expect `{"handle":"...","token":"bbln_..."}`. 403 = you're not the owner / not reaching it via tailscale-serve; 409 = handle already exists (rotate on the host instead: `babylon-server rotate-token --handle <username>`).
2. Write a **project-local, 0600 `.mcp.json`** in this repo with the token inlined — this shadows the plugin's global babylon server for THIS directory, giving each repo its own identity. Do NOT use a global `BABYLON_TOKEN` env var: Claude Code expands `${VAR}` only from the OS environment, so a single global var makes every repo the same agent. Use `jq` so the token never reaches shell history or chat output:
   `jq -n --arg tok "<token>" '{mcpServers:{babylon:{type:"http",url:"https://babylon.taild4189d.ts.net/mcp",headers:{Authorization:("Bearer " + $tok)}}}}' > .mcp.json && chmod 600 .mcp.json`
3. Ensure `.mcp.json` can never be committed. Run `git rev-parse --is-inside-work-tree` first — do this even if the directory looks like it isn't a git repo, it's easy to be wrong. If it is a repo and `git check-ignore .mcp.json` finds nothing, append `.mcp.json` to `.gitignore`.
4. Tell the user to **restart Claude Code here**, then **approve the `babylon` server** when prompted, then run `/babylon` to register and catch up.
Do not echo the token into the chat beyond writing the file.
