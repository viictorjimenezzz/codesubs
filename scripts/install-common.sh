#!/bin/zsh
# Shared installer helpers; sourcing has no process or filesystem side effects.
app_pids() {
  local executable="$1" match_cli_name="${2:-0}"
  ps -axwwo pid=,comm= | SM_INSTALL_EXECUTABLE="$executable" SM_INSTALL_MATCH_CLI_NAME="$match_cli_name" python3 -c '
import os, sys
expected = os.path.abspath(os.environ["SM_INSTALL_EXECUTABLE"])
match_cli_name = os.environ["SM_INSTALL_MATCH_CLI_NAME"] == "1"
for line in sys.stdin:
    fields = line.strip().split(None, 1)
    if len(fields) != 2:
        continue
    pid, command = fields
    # A CLI launched through PATH can appear as a bare command name.
    from_path = not os.path.isabs(command) and os.path.basename(command) == os.path.basename(expected)
    if command == expected or (match_cli_name and from_path):
        print(pid)
'
}

quit_app() {
  local bundle_id="$1" executable="$2" survivors pid
  survivors="$(app_pids "$executable")"
  [[ -z "$survivors" ]] && return 0
  osascript -e "tell application id \"$bundle_id\" to quit" 2>/dev/null || true
  for _ in {1..100}; do
    survivors="$(app_pids "$executable")"
    [[ -z "$survivors" ]] && return 0
    sleep 0.1
  done
  for pid in ${(f)survivors}; do kill -TERM "$pid" 2>/dev/null || true; done
  for _ in {1..100}; do
    survivors="$(app_pids "$executable")"
    [[ -z "$survivors" ]] && return 0
    sleep 0.1
  done
  print -u2 "Cannot upgrade while $executable is still running (PIDs: $survivors). Quit it and rerun the installer."
  return 1
}

require_idle_cli() {
  local executable="$1" active
  active="$(app_pids "$executable" 1)"
  if [[ -n "$active" ]]; then
    print -u2 "A command from $executable is still running (PIDs: $active). Wait for it to finish or cancel it before upgrading."
    return 1
  fi
}
