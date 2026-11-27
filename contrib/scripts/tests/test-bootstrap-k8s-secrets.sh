#!/usr/bin/env bash
# Tests bootstrap-k8s-secrets.sh against the fake kubectl in bin/. The tests
# need bash, python3, and openssl. They make no cluster requests.
set -uo pipefail

TESTS="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$TESTS/../bootstrap-k8s-secrets.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILED=0
CASES=0
STATE=""

OP=(OP_SERVICE_ACCOUNT_TOKEN=op-token OP_VAULT=vault)
SLACK_NOTE="has no Slack keys"
OP_NOTE="has no 1Password token"

# Starts a case with a new, empty cluster.
fresh() {
  CASES=$((CASES + 1))
  STATE="$WORK/cluster-$CASES"
  echo "case: $1"
}

# Starts a case on the cluster of the previous case.
next() {
  echo "case: $1"
}

# Runs the script with only the given variables in its environment.
bootstrap() {
  env -i PATH="$TESTS/bin:$PATH" HOME="$WORK" FAKE_KUBE_STATE="$STATE" "$@" \
    bash "$SCRIPT" --namespace test >"$WORK/out" 2>"$WORK/err"
}

# Prints the decoded value of a key of centaur-infra-env.
value() {
  python3 -I - "$STATE/centaur-infra-env.json" "$1" <<'EOF'
import base64, json, pathlib, sys
secret = pathlib.Path(sys.argv[1])
data = json.loads(secret.read_text()) if secret.exists() else {}
value = data.get(sys.argv[2])
print("<absent>" if value is None else base64.b64decode(value).decode())
EOF
}

secret_exists() {
  [[ -e "$STATE/centaur-infra-env.json" ]] && echo yes || echo no
}

# Prints how many times the script printed the note $1.
notes() {
  grep -c "$1" "$WORK/err"
}

check() {
  if [[ "$2" == "$3" ]]; then
    echo "  ok: $1"
  else
    echo "  FAIL: $1: expected [$3], got [$2]"
    FAILED=1
  fi
}

fresh "no Slack variables create a Secret without Slack keys"
bootstrap "${OP[@]}"
check "exit status" "$?" 0
check "OP_VAULT" "$(value OP_VAULT)" vault
check "SLACK_BOT_TOKEN" "$(value SLACK_BOT_TOKEN)" "<absent>"
check "SLACK_SIGNING_SECRET" "$(value SLACK_SIGNING_SECRET)" "<absent>"
check "SLACKBOT_API_KEY" "$(value SLACKBOT_API_KEY)" "<absent>"
check "note" "$(notes "$SLACK_NOTE")" 1

next "a later run adds the Slack keys to the Secret"
bootstrap "${OP[@]}" SLACK_BOT_TOKEN=xoxb-1 SLACK_SIGNING_SECRET=signing-1
check "exit status" "$?" 0
check "SLACK_BOT_TOKEN" "$(value SLACK_BOT_TOKEN)" xoxb-1
check "SLACK_SIGNING_SECRET" "$(value SLACK_SIGNING_SECRET)" signing-1
generated="$(value SLACKBOT_API_KEY)"
check "generated SLACKBOT_API_KEY" "$(grep -cE '^[0-9a-f]{64}$' <<<"$generated")" 1
check "note" "$(notes "$SLACK_NOTE")" 0

next "a later run replaces the Slack credentials and keeps the api-rs key"
bootstrap "${OP[@]}" SLACK_BOT_TOKEN=xoxb-2 SLACK_SIGNING_SECRET=signing-2 SLACKBOT_API_KEY=other
check "exit status" "$?" 0
check "SLACK_BOT_TOKEN" "$(value SLACK_BOT_TOKEN)" xoxb-2
check "SLACK_SIGNING_SECRET" "$(value SLACK_SIGNING_SECRET)" signing-2
check "SLACKBOT_API_KEY" "$(value SLACKBOT_API_KEY)" "$generated"

next "a run without Slack variables keeps the Slack keys"
bootstrap "${OP[@]}"
check "exit status" "$?" 0
check "SLACK_BOT_TOKEN" "$(value SLACK_BOT_TOKEN)" xoxb-2
check "SLACKBOT_API_KEY" "$(value SLACKBOT_API_KEY)" "$generated"
check "note" "$(notes "$SLACK_NOTE")" 0

fresh "the Slack variables seed the Slack keys of a new Secret"
bootstrap "${OP[@]}" SLACK_BOT_TOKEN=xoxb-1 SLACK_SIGNING_SECRET=signing-1 SLACKBOT_API_KEY=given
check "exit status" "$?" 0
check "SLACK_BOT_TOKEN" "$(value SLACK_BOT_TOKEN)" xoxb-1
check "SLACK_SIGNING_SECRET" "$(value SLACK_SIGNING_SECRET)" signing-1
check "SLACKBOT_API_KEY" "$(value SLACKBOT_API_KEY)" given
check "note" "$(notes "$SLACK_NOTE")" 0

fresh "a token without a signing secret stops the script"
bootstrap "${OP[@]}" SLACK_BOT_TOKEN=xoxb-1
check "exit status" "$?" 1
check "error" "$(cat "$WORK/err")" "FATAL: SLACK_SIGNING_SECRET is required in the shell environment"
check "Secret" "$(secret_exists)" no

fresh "a signing secret without a token stops the script"
bootstrap "${OP[@]}" SLACK_SIGNING_SECRET=signing-1
check "exit status" "$?" 1
check "error" "$(cat "$WORK/err")" "FATAL: SLACK_BOT_TOKEN is required in the shell environment"
check "Secret" "$(secret_exists)" no

fresh "no variables create a Secret without 1Password keys"
bootstrap
check "exit status" "$?" 0
check "OP_SERVICE_ACCOUNT_TOKEN" "$(value OP_SERVICE_ACCOUNT_TOKEN)" "<absent>"
check "OP_VAULT" "$(value OP_VAULT)" "<absent>"
check "generated CENTAUR_JWT_SIGNING_SECRET" "$(grep -cE '^[0-9a-f]{128}$' <<<"$(value CENTAUR_JWT_SIGNING_SECRET)")" 1
check "1Password note" "$(notes "$OP_NOTE")" 1
check "Slack note" "$(notes "$SLACK_NOTE")" 1

next "a 1Password token for a Secret without a vault needs OP_VAULT"
bootstrap OP_SERVICE_ACCOUNT_TOKEN=op-token
check "exit status" "$?" 1
check "error" "$(cat "$WORK/err")" "FATAL: OP_VAULT is required in the shell environment"
check "OP_SERVICE_ACCOUNT_TOKEN" "$(value OP_SERVICE_ACCOUNT_TOKEN)" "<absent>"

next "a later run adds the 1Password keys to the Secret"
bootstrap "${OP[@]}"
check "exit status" "$?" 0
check "OP_SERVICE_ACCOUNT_TOKEN" "$(value OP_SERVICE_ACCOUNT_TOKEN)" op-token
check "OP_VAULT" "$(value OP_VAULT)" vault
check "1Password note" "$(notes "$OP_NOTE")" 0

next "a token for a Secret with a vault needs no OP_VAULT"
bootstrap OP_SERVICE_ACCOUNT_TOKEN=op-token-2
check "exit status" "$?" 0
check "OP_SERVICE_ACCOUNT_TOKEN" "$(value OP_SERVICE_ACCOUNT_TOKEN)" op-token-2
check "OP_VAULT" "$(value OP_VAULT)" vault

fresh "a service account token without a vault stops the script"
bootstrap OP_SERVICE_ACCOUNT_TOKEN=op-token
check "exit status" "$?" 1
check "error" "$(cat "$WORK/err")" "FATAL: OP_VAULT is required in the shell environment"
check "Secret" "$(secret_exists)" no

fresh "a Connect token without a vault stops the script"
bootstrap OP_CONNECT_TOKEN=connect-token
check "exit status" "$?" 1
check "error" "$(cat "$WORK/err")" "FATAL: OP_VAULT is required in the shell environment"
check "Secret" "$(secret_exists)" no

fresh "a Connect token and a vault need no service account token"
bootstrap OP_CONNECT_TOKEN=connect-token OP_VAULT=vault
check "exit status" "$?" 0
check "OP_CONNECT_TOKEN" "$(value OP_CONNECT_TOKEN)" connect-token
check "OP_VAULT" "$(value OP_VAULT)" vault
check "OP_SERVICE_ACCOUNT_TOKEN" "$(value OP_SERVICE_ACCOUNT_TOKEN)" "<absent>"
check "1Password note" "$(notes "$OP_NOTE")" 0

if [[ "$FAILED" == 1 ]]; then
  echo "bootstrap-k8s-secrets.sh tests failed" >&2
fi
exit "$FAILED"
