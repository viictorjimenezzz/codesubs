#!/bin/sh
set -eu
[ "$PWD" -ef "$CLAUDE_CONFIG_DIR" ]
[ -z "${CLAUDE_CODE_OAUTH_TOKEN+x}${ANTHROPIC_API_KEY+x}${ANTHROPIC_AUTH_TOKEN+x}" ]
case "$*" in
    "auth status --json")
        dd if=/dev/zero bs=65536 count=2 >&2 2>/dev/null
        printf '{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","email":"claude@example.test"}\n'
        ;;
    "-p /usage --output-format json --no-session-persistence --setting-sources project")
        printf '%s\n' '{"result":"Current session: 1.5% used\nCurrent week (all models): 9% used\nCurrent week (Sonnet): 15% used","is_error":false}'
        ;;
    "auth login --claudeai --email claude@example.test")
        # Leave the URL temporarily unterminated. The parent must wait for the
        # full value, including the second query parameter, before emitting it.
        printf '%s' 'https://example.test/cai/oauth/authorize?code=true' >&2
        sleep 0.05
        printf '%s\n' '&state=abcd' >&2
        IFS= read -r code
        [ "$code" = 'fake-code' ]
        printf 'signed in\n' > "$CLAUDE_CONFIG_DIR/login-complete"
        ;;
    *) exit 11 ;;
esac
