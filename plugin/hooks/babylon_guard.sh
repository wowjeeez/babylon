#!/usr/bin/env bash
set -u

[ "${BABYLON_GUARD:-1}" = "0" ] && exit 0

input=$(cat)
tool=$(printf '%s' "$input" | jq -r '.tool_name // empty' 2>/dev/null)
[ -z "$tool" ] && exit 0

DENY_GIT=(
  'git( +[^ ]+)* +push( +[^ ]+)* +(-f|--force|--force-with-lease)'
  'git( +[^ ]+)* +push( +[^ ]+)* +\+[^ :]+:'
  'git( +[^ ]+)* +reset( +[^ ]+)* +--hard'
  'git( +[^ ]+)* +rebase'
  'git( +[^ ]+)* +filter-branch'
  'git +filter-repo'
  'git( +[^ ]+)* +clean( +[^ ]+)* +-[a-zA-Z]*[dfx]'
)
DENY_FS=(
  'rm( +-[a-zA-Z]+)* +-[a-zA-Z]*r[a-zA-Z]*f'
  'rm( +-[a-zA-Z]+)* +-[a-zA-Z]*f[a-zA-Z]*r'
  'rm +(-[a-zA-Z]+ +)*-[a-zA-Z]*r[a-zA-Z]* +.*(--force|-f)'
  'rm +(-[a-zA-Z]+ +)*-[a-zA-Z]*r[a-zA-Z]* +(/|~|\.\.|\*|\$HOME)( |$|/)'
  'find +.*-delete'
  'dd +.*of='
  'mkfs'
  'shred '
  'truncate '
  '> +/dev/sd'
)
DENY_SECRETS=(
  '(cat|cp|scp|rsync|base64|xxd|strings|tee|less|more) +.*(id_rsa|id_ed25519|\.pem|\.p12|\.pfx|\.aws/credentials|\.config/gcloud)'
)
DENY_INFRA=(
  'terraform( +[^ ]+)* +(apply|destroy)'
  'kubectl( +[^ ]+)* +(apply|delete|edit|scale|drain|cordon|replace|patch)'
  'helm( +[^ ]+)* +(install|upgrade|uninstall|delete|rollback)'
  'ansible-playbook'
  'pulumi( +[^ ]+)* +(up|destroy)'
  'docker( +[^ ]+)* +push'
  'systemctl( +[^ ]+)* +(start|stop|restart|enable|disable|mask)'
  '(aws|gcloud|az)( +[^ ]+)* +[a-z-]*(create|delete|deploy|terminate|destroy)'
  '(^|[ /])[a-zA-Z0-9_.-]*deploy[a-zA-Z0-9_.-]*\.(sh|py|rb|js|ts)( |$)'
  'make +([^ ]+ +)*deploy'
)
MCP_DENY=(
  '_delete$'
  '_destroy$'
  'delete_'
  'pause_'
  'restore_'
  'reset_branch'
  'apply_migration'
  'deploy_'
)

deny() {
  jq -cn --arg r "$1" '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

tail_msg="— gated destructive op. Surface it to the human (who can run it directly via ! or set BABYLON_GUARD=0). Do not retry or route around it."

if [ "$tool" = "Bash" ]; then
  cmd=$(printf '%s' "$input" | jq -r '.tool_input.command // empty' 2>/dev/null)
  [ -z "$cmd" ] && exit 0
  for pat in "${DENY_GIT[@]}" "${DENY_FS[@]}" "${DENY_SECRETS[@]}" "${DENY_INFRA[@]}"; do
    if printf '%s' "$cmd" | grep -Eq -- "$pat"; then
      deny "babylon guard blocked: \`$cmd\` $tail_msg"
    fi
  done
  exit 0
fi

case "$tool" in
  mcp__*)
    for pat in "${MCP_DENY[@]}"; do
      if printf '%s' "$tool" | grep -Eq -- "$pat"; then
        deny "babylon guard blocked MCP tool \`$tool\` $tail_msg"
      fi
    done
    sql=$(printf '%s' "$input" | jq -r '.tool_input.query // .tool_input.sql // empty' 2>/dev/null)
    if [ -n "$sql" ] && printf '%s' "$sql" | grep -Eqi 'drop +|delete +|truncate +|alter +'; then
      deny "babylon guard blocked destructive SQL via \`$tool\` $tail_msg"
    fi
    exit 0
    ;;
esac
exit 0
