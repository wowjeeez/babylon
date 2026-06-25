#!/usr/bin/env bash
set -u

input=$(cat)

event=$(printf '%s' "$input" | jq -r '.hook_event_name // empty' 2>/dev/null)
stop_active=$(printf '%s' "$input" | jq -r '.stop_hook_active // false' 2>/dev/null)

if [ "$stop_active" = "true" ]; then
  exit 0
fi

lines=$(python3 "$CLAUDE_PLUGIN_ROOT/scripts/babylon_unread.py" 2>/dev/null)

if [ -z "$lines" ]; then
  exit 0
fi

if [ "$event" = "Stop" ]; then
  reason="🔔 New babylon items addressed to you:
${lines}

Handle them now per the babylon auto-act protocol: answer questions from context; for tasks/issues addressed to you DO the work (edit code, run tests, commit, push, open a PR) all autonomously, then resolve/close with a summary. Routine work incl. push is autonomous; if YOU judge something destructive or irreversible (rm -rf, force-push, wiping data, infra teardown, secrets, outbound), ask the human for approval first. ack everything you process."
  jq -n --arg reason "$reason" '{decision:"block",reason:$reason}'
else
  ac="🔔 Unread babylon items:
${lines}
Run your babylon auto-act sweep — answer questions and do tasks/issues addressed to you (code, tests, commit, push, open a PR) autonomously, then resolve/close. Ask the human before anything you judge destructive (rm -rf, force-push, wiping data, infra, secrets, outbound)."
  jq -n --arg ac "$ac" '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$ac}}'
fi

exit 0
