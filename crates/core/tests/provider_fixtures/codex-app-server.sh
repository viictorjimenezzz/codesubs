#!/bin/sh
set -eu
[ "$#" -eq 2 ] && [ "$1" = app-server ] && [ "$2" = --stdio ]
[ "$PWD" -ef "$CODEX_HOME" ]
printf '%s\n' "$$" >> "$CODEX_HOME/sessions.log"

while IFS= read -r request; do
    id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            case "$request" in
                *'"experimentalApi":true'*) ;;
                *) exit 11 ;;
            esac
            printf 'app server banner\n'
            printf '{"id":7000,"method":"unsupported/request","params":{}}\n'
            printf '{"id":%s,"result":{}}\n' "$id"
            ;;
        *'"code":-32601'*'"id":7000'*) printf 'handled\n' >> "$CODEX_HOME/rpc.log" ;;
        *'"method":"initialized"'*) printf 'initialized\n' >> "$CODEX_HOME/rpc.log" ;;
        *'"method":"account/read"'*'"refreshToken":true'*)
            printf 'forced\n' >> "$CODEX_HOME/rpc.log"
            printf '{"id":%s,"result":{"account":null}}\n' "$id"
            ;;
        *'"method":"account/read"'*'"refreshToken":false'*)
            printf 'normal\n' >> "$CODEX_HOME/rpc.log"
            printf '{"id":%s,"result":{"account":{"type":"chatgpt","email":"codex@example.test","planType":"plus"}}}\n' "$id"
            ;;
        *'"method":"account/rateLimits/read"'*)
            printf '{"id":%s,"result":{"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":23,"windowDurationMins":300},"secondary":{"usedPercent":67,"windowDurationMins":10080},"credits":{"balance":"2.5","hasCredits":true}}},"rateLimitResetCredits":{"availableCount":1}}}\n' "$id"
            ;;
        *'"method":"account/login/start"'*)
            if [ ! -e pending-login ]; then
                printf '{"method":"account/login/completed","params":{"loginId":"other-login","success":false}}\n'
                printf '{"method":"account/login/completed","params":{"loginId":"test-login","success":true}}\n'
            fi
            printf '{"id":%s,"result":{"loginId":"test-login","authUrl":"https://example.test/codex/login"}}\n' "$id"
            ;;
        *'"method":"account/login/cancel"'*'"loginId":"test-login"'*)
            printf 'cancelled\n' >> "$CODEX_HOME/rpc.log"
            printf '{"id":%s,"result":{"status":"canceled"}}\n' "$id"
            ;;
        *'"method":"account/rateLimitResetCredit/consume"'*'"idempotencyKey":"12345678-1234-1234-1234-123456789012"'*)
            printf '{"id":%s,"result":{"outcome":"alreadyRedeemed"}}\n' "$id"
            ;;
        *) exit 12 ;;
    esac
done
