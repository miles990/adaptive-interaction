#!/usr/bin/env bash
# Fake Claude Code CLI that floods stderr (D16 regression fixture).
#
# Owned by the gateway crate on purpose: the runtime crate has its own copy of
# fake_claude.sh and a test must never depend on another crate's fixture path.
#
# Behaviour: announce a session on stdout, dump ~5 MB to stderr, then print two
# lines that deliberately contain secrets (an Authorization header and a home
# path) so the test can prove the retained tail is redacted, then claim a
# result and exit 0. It never reads stdin, so no prompt is required.
case "$1" in
  --version) echo "fake-claude-flood 1.0.0 (Claude Code)"; exit 0 ;;
  auth) echo '{"loggedIn": true, "authMethod": "fake"}'; exit 0 ;;
esac

echo '{"type":"system","subtype":"init","session_id":"flood-session","model":"fake-model-x"}'

LINE=$(printf '%*s' 1000 '' | tr ' ' 'x')
i=1
while [ "$i" -le 5000 ]; do
  printf 'stderr line %s %s\n' "$i" "$LINE" >&2
  i=$((i + 1))
done

# The secrets land at the very end, i.e. inside the window the tail keeps.
echo 'Authorization: Bearer abc123def456' >&2
echo 'workdir=/Users/someone/secret' >&2

echo '{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.01,"num_turns":1}'
exit 0
