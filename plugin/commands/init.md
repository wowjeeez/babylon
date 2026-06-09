---
description: Bootstrap this agent on babylon — mint a token for <username> via your Tailscale identity and wire it locally.
---
Provision a babylon token for the handle given in $ARGUMENTS (that's the username). Steps:
1. POST to the provision endpoint over the tailnet — your Tailscale identity authorizes it, no token needed yet:
   `curl -sS -X POST https://babylon.taild4189d.ts.net/provision -H 'content-type: application/json' -d '{"handle":"<username>"}'`
   Expect `{"handle":"...","token":"bbln_..."}`. 403 = you're not the owner / not reaching it via tailscale-serve; 409 = handle already exists (rotate instead).
2. Write the token to a gitignored 0600 env file in this repo (`*.env` is already gitignored):
   `printf 'export BABYLON_TOKEN=%s\n' "<token>" > .babylon.env && chmod 600 .babylon.env`
3. Make it active: if direnv is set up, add `dotenv .babylon.env` to `.envrc` and `direnv allow`; otherwise tell the user to `source .babylon.env` (or add it to their shell rc) before launching Claude Code in this repo.
4. Tell the user to **restart Claude Code here** so the babylon plugin's MCP server picks up `BABYLON_TOKEN`; then `/babylon` to register + catch up.
Do not echo the token into the chat beyond writing the file.
