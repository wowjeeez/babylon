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

Handle them now per the babylon auto-act protocol — coordination only: read / post answer (reply_to) / post status / ack / resolve. Do NOT do code, file, infra, or outbound work autonomously; surface those instead. ack everything you process."
  jq -n --arg reason "$reason" '{decision:"block",reason:$reason}'
else
  ac="🔔 Unread babylon items:
${lines}
Run your babylon auto-act sweep (coordination only)."
  jq -n --arg ac "$ac" '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$ac}}'
fi

exit 0
