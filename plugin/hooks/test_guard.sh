#!/usr/bin/env bash
set -u
GUARD="$(cd "$(dirname "$0")" && pwd)/babylon_guard.sh"
fail=0

decision() {
  local input
  input=$(cat)
  local out
  out=$(printf '%s' "$input" | bash "$GUARD")
  if printf '%s' "$out" | grep -q '"permissionDecision":"deny"'; then
    printf 'deny'
  elif printf '%s' "$out" | grep -q '"permissionDecision":"ask"'; then
    printf 'ask'
  else
    printf 'allow'
  fi
}

bash_case() {
  local expect="$1" cmd="$2"
  local got
  got=$(jq -n --arg c "$cmd" '{tool_name:"Bash",tool_input:{command:$c}}' | decision)
  if [ "$got" = "$expect" ]; then echo "ok   [$expect] $cmd"; else echo "FAIL [want $expect got $got] $cmd"; fail=1; fi
}

mcp_case() {
  local expect="$1" tool="$2"
  local got
  got=$(jq -n --arg t "$tool" '{tool_name:$t,tool_input:{}}' | decision)
  if [ "$got" = "$expect" ]; then echo "ok   [$expect] $tool"; else echo "FAIL [want $expect got $got] $tool"; fail=1; fi
}

bash_case deny  'git push --force'
bash_case deny  'git push -f origin main'
bash_case deny  'git push --force-with-lease'
bash_case deny  'git reset --hard HEAD~3'
bash_case deny  'git rebase -i main'
bash_case deny  'git filter-branch --tree-filter x'
bash_case deny  'git clean -fdx'
bash_case deny  'rm -rf build'
bash_case deny  'rm -fr /tmp/x'
bash_case deny  'rm -r --force node_modules'
bash_case deny  'rm -r /'
bash_case deny  "find . -name '*.log' -delete"
bash_case deny  'dd if=/dev/zero of=/dev/sda'
bash_case deny  'terraform apply'
bash_case deny  'terraform destroy -auto-approve'
bash_case deny  'kubectl delete pod x'
bash_case deny  'helm upgrade app .'
bash_case deny  'ansible-playbook site.yml'
bash_case deny  'docker push myimage'
bash_case deny  'systemctl restart nginx'
bash_case deny  'aws s3 delete-bucket --bucket x'
bash_case deny  'cat ~/.ssh/id_rsa'
bash_case deny  'base64 secrets/key.pem'

bash_case ask   'git push'
bash_case ask   'git push origin feature'
bash_case allow 'git commit -m "x"'
bash_case allow 'rm build.log'
bash_case allow 'rm -r node_modules'
bash_case allow 'find . -name "*.log"'
bash_case allow 'terraform plan'
bash_case allow 'kubectl get pods'
bash_case allow 'docker build -t x .'
bash_case allow 'cat README.md'
bash_case allow 'ssh-add ~/.ssh/id_rsa'
bash_case allow 'cargo test --workspace'

mcp_case deny  'mcp__claude_ai_Cloudflare__d1_database_delete'
mcp_case deny  'mcp__claude_ai_Cloudflare__kv_namespace_delete'
mcp_case deny  'mcp__claude_ai_Supabase__deploy_edge_function'
mcp_case deny  'mcp__claude_ai_Supabase__pause_project'
mcp_case deny  'mcp__claude_ai_Supabase__apply_migration'
mcp_case allow 'mcp__claude_ai_Cloudflare__kv_namespaces_list'
mcp_case allow 'mcp__babylon__post'
mcp_case allow 'mcp__babylon__file_issue'

sql_drop=$(jq -n '{tool_name:"mcp__claude_ai_Supabase__execute_sql",tool_input:{query:"DROP TABLE x"}}' | decision)
[ "$sql_drop" = deny ] && echo "ok   [deny] execute_sql DROP" || { echo "FAIL execute_sql DROP not denied"; fail=1; }
sql_sel=$(jq -n '{tool_name:"mcp__claude_ai_Supabase__execute_sql",tool_input:{query:"SELECT 1"}}' | decision)
[ "$sql_sel" = allow ] && echo "ok   [allow] execute_sql SELECT" || { echo "FAIL execute_sql SELECT denied"; fail=1; }

esc=$(jq -n '{tool_name:"Bash",tool_input:{command:"git push --force"}}' | BABYLON_GUARD=0 bash "$GUARD")
printf '%s' "$esc" | grep -q deny && { echo "FAIL escape hatch did not disable guard"; fail=1; } || echo "ok   [allow] BABYLON_GUARD=0 disables guard"

[ "$fail" = 0 ] && echo "ALL GUARD TESTS PASSED" || echo "GUARD TESTS FAILED"
exit "$fail"
