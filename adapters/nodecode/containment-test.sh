#!/usr/bin/env bash
# Deterministic containment test for meshloop-nodecode-sandbox (issue #138).
#
# Needs bash, git and bubblewrap with unprivileged user namespaces. Needs no nodecode,
# no model and no network: every probe runs under `--run-command`, which builds the
# same sandbox a nodecode worker gets, minus the read-only nodecode install.
#
#   adapters/nodecode/containment-test.sh [BASE_DIR]
#
# BASE_DIR must be on ext4, xfs or btrfs (default: <repo>/.agent-runs). The test makes a
# disposable repository with a linked worktree under it and removes it on exit.
# Exit 0: passed, or skipped with a "SKIP:" line (not Linux, no bwrap, no user
# namespaces). Exit 1: a containment check failed, or a skip while
# MESHLOOP_REQUIRE_CONTAINMENT=1 (CI sets it where a probe shows bwrap works).
# The driver checks need /usr/bin/python3 and are skipped, with a line, without it.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
wrapper="$here/meshloop-nodecode-sandbox"
repo_root="$(cd -- "$here/../.." && pwd -P)"

skip() {
  if [[ "${MESHLOOP_REQUIRE_CONTAINMENT:-}" == "1" ]]; then
    printf 'FAIL: containment test required (MESHLOOP_REQUIRE_CONTAINMENT=1) but would skip: %s\n' "$*"
    exit 1
  fi
  printf 'SKIP: %s\n' "$*"
  exit 0
}

[[ "$(uname -s)" == "Linux" ]] || skip "containment is Linux/WSL only"
command -v bwrap >/dev/null 2>&1 || skip "bwrap (bubblewrap) is not installed"
command -v git >/dev/null 2>&1 || skip "git is not installed"
# The namespace flags the wrapper uses; --disable-userns needs bubblewrap 0.8 or newer.
if ! probe_err="$(bwrap --unshare-user --disable-userns --unshare-pid --unshare-ipc --unshare-uts \
  --unshare-cgroup-try --die-with-parent --new-session --ro-bind / / true 2>&1)"; then
  skip "bwrap cannot build the sandbox here (user namespaces or bubblewrap >= 0.8 missing): $probe_err"
fi

base="${1:-$repo_root/.agent-runs}"
mkdir -p -- "$base"
case "$(stat -f -c %T -- "$base")" in
  ext2/ext3 | xfs | btrfs) ;;
  *) skip "BASE_DIR $base is not on ext4, xfs or btrfs, which the wrapper requires" ;;
esac
work="$(mktemp -d "$base/containment-test.XXXXXXXX")"
marker="meshloop-containment-$(basename -- "$work")"
real_home="$(getent passwd "$(id -u)" | cut -d: -f6)"
probe_name=".meshloop-containment-probe-$$"
failures=0
sandbox_pid=""

cleanup() {
  if [[ -n "$sandbox_pid" ]]; then kill -KILL -- "-$sandbox_pid" 2>/dev/null || true; fi
  # A failed process check must not leave its probes running.
  pkill -KILL -f -- "$marker" 2>/dev/null || true
  # A probe that escaped would have left a file; remove it either way.
  rm -f -- "$real_home/$probe_name" "$real_home/.nodecode/$probe_name" "/tmp/$probe_name" 2>/dev/null || true
  rm -rf -- "$work"
}
trap cleanup EXIT

pass() { printf 'PASS: %s\n' "$*"; }
fail() {
  printf 'FAIL: %s\n' "$*"
  failures=$((failures + 1))
}

# --- layout: a repository, a linked worktree inside it (meshloop's default layout),
# and a fake operator home holding a nodecode home and an ssh key.
repo="$work/repo"
fake_home="$work/home"
git_q() { env -i PATH=/usr/bin:/bin HOME="$fake_home" GIT_CONFIG_NOSYSTEM=1 git "$@" >/dev/null 2>&1; }
mkdir -p "$repo" "$fake_home/.nodecode" "$fake_home/.ssh"
printf 'organism\n' >"$fake_home/.nodecode/config.jsonc"
printf 'not-a-key\n' >"$fake_home/.ssh/id_test"
git_q -C "$repo" init -q
printf 'seed\n' >"$repo/README.md"
git_q -C "$repo" add README.md
git_q -C "$repo" -c user.email=t@t -c user.name=t commit -q -m seed
git_q -C "$repo" worktree add -q "$repo/.meshloop/worktrees/w1" -b w1
wt="$repo/.meshloop/worktrees/w1"
[[ -f "$wt/.git" ]] || { echo "FAIL: could not create a linked worktree"; exit 1; }

canary_value="canary-$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')"
cred_file="$work/canary.key"
(umask 077 && printf '%s\n' "$canary_value" >"$cred_file")

# Runs a probe inside the sandbox: the wrapper with the fake home as HOME, the caller's
# environment plus a canary variable, the worktree as working directory.
in_sandbox() {
  (cd "$wt" && HOME="$fake_home" MESHLOOP_CANARY_ENV="$canary_value" \
    "$wrapper" --credential-provider canary --credential-file "$cred_file" \
    --run-command -- "$@")
}

# --- 1. writes ----------------------------------------------------------------
# Each target: try the write inside the sandbox, then check the host.
write_probe='for t in "$@"; do if ( : >"$t" ) 2>/dev/null; then echo "wrote $t"; else echo "refused $t"; fi; done'
targets=(
  "$wt/$probe_name"                      # the worktree: must succeed
  "$repo/$probe_name"                    # the repository root above the worktree
  "$repo/.git/$probe_name"               # the git common dir (bound read-only)
  "$fake_home/$probe_name"               # the operator's home (the HOME given)
  "$fake_home/.nodecode/$probe_name"     # the operator's nodecode home
  "$real_home/$probe_name"               # the real operator home
  "$real_home/.nodecode/$probe_name"     # the real operator nodecode home
  "/tmp/$probe_name"                     # /tmp: the sandbox's own tmpfs
  "/usr/$probe_name"
  "/etc/$probe_name"
  "/$probe_name"
)
out="$(in_sandbox sh -c "$write_probe" sh "${targets[@]}")" || true

if [[ -f "$wt/$probe_name" ]] && grep -qxF "wrote $wt/$probe_name" <<<"$out"; then
  pass "worker writes inside its worktree"
else
  fail "worker could not write inside its worktree: $out"
fi
for t in "${targets[@]:1}"; do
  if [[ "$t" == "/tmp/$probe_name" ]]; then
    # /tmp is a private tmpfs: the write may succeed in the sandbox, never on the host.
    [[ ! -e "$t" ]] && pass "write to /tmp stays in the sandbox's own tmpfs" \
      || fail "a sandbox write reached the host's /tmp"
    continue
  fi
  if [[ -e "$t" ]]; then
    fail "write escaped to the host: $t"
  elif grep -qxF "refused $t" <<<"$out"; then
    pass "write refused: $t"
  else
    fail "no refusal recorded for $t: $out"
  fi
done

# The worktree's .git file: read-only, so the worker cannot point the host's later git
# commands at a gitdir (and config) of its own.
gitfile_probe='{ printf "gitdir: /tmp/evil\n" >"$1/.git"; } 2>/dev/null && echo rewritten || echo refused-write
rm -f "$1/.git" 2>/dev/null && echo removed || echo refused-remove
mv "$1/.git" "$1/moved" 2>/dev/null && echo moved || echo refused-move
git -C "$1" rev-parse --is-inside-work-tree 2>/dev/null || echo git-unusable'
before_git="$(cat "$wt/.git")"
out="$(in_sandbox sh -c "$gitfile_probe" sh "$wt")" || true
if [[ "$(cat "$wt/.git" 2>/dev/null)" == "$before_git" ]] \
  && grep -qx refused-write <<<"$out" && grep -qx refused-remove <<<"$out" && grep -qx refused-move <<<"$out"; then
  pass "the worktree's .git file cannot be rewritten, removed or moved"
else
  fail "the worktree's .git file is writable by the worker: $out"
fi
grep -qx true <<<"$out" && pass "git works read-only inside the worktree" \
  || fail "git does not work inside the worktree: $out"

# A forged `.git` (#181 review): an admin dir that is not <common>/worktrees/<name>
# binds no git dir, so its commondir pointing at a fake ~/.ssh exposes nothing; an admin
# dir that follows the layout inside a secret store is refused.
forged="$work/forged-admin"
mkdir -p "$forged"
printf '%s\n' "$wt/.git" >"$forged/gitdir"
printf '%s\n' "$fake_home/.ssh" >"$forged/commondir"
printf 'gitdir: %s\n' "$forged" >"$wt/.git"
out="$(in_sandbox sh -c 'for p in "$@"; do [ -e "$p" ] && echo "visible $p" || echo "absent $p"; done' sh \
  "$fake_home/.ssh/id_test" "$forged")" || true
if grep -qxF "absent $fake_home/.ssh/id_test" <<<"$out" && grep -qxF "absent $forged" <<<"$out"; then
  pass "a forged .git with commondir at ~/.ssh binds no git dir"
else
  fail "a forged .git exposed a git dir: $out"
fi
mkdir -p "$fake_home/.ssh/worktrees/w1"
printf '%s\n' "$wt/.git" >"$fake_home/.ssh/worktrees/w1/gitdir"
printf '../..\n' >"$fake_home/.ssh/worktrees/w1/commondir"
printf 'gitdir: %s\n' "$fake_home/.ssh/worktrees/w1" >"$wt/.git"
rc=0
msg="$(cd "$wt" && HOME="$fake_home" "$wrapper" --run-command -- true 2>&1)" || rc=$?
if ((rc == 125)) && grep -qF "secret store" <<<"$msg"; then
  pass "refused: git dirs inside a secret store"
else
  fail "git dirs inside a secret store not refused ($rc): $msg"
fi
printf '%s\n' "$before_git" >"$wt/.git"
rm -rf -- "$fake_home/.ssh/worktrees" "$forged"

# --- 2. visibility: the operator's homes and secrets are not mounted -----------------
# Each path must exist on the host, or the check would prove nothing. The fake home's
# paths always do; a real-home path that is absent on this host is reported, not passed.
vis_probe='for p in "$@"; do if [ -e "$p" ]; then echo "visible $p"; else echo "absent $p"; fi; done'
hidden=("$fake_home/.nodecode" "$fake_home/.ssh" "$repo/README.md" "$real_home")
for p in "$real_home/.nodecode" "$real_home/.ssh" "$real_home/.config"; do
  if [[ -e "$p" ]]; then hidden+=("$p"); else printf 'NOTE: %s is absent on this host; not checked\n' "$p"; fi
done
# $real_home itself may be an ancestor of the worktree (then it exists in the sandbox as
# an empty mount point); check its listing instead of its existence.
out="$(in_sandbox sh -c "$vis_probe" sh "${hidden[@]}")" || true
for p in "${hidden[@]}"; do
  [[ "$p" == "$real_home" ]] && continue
  if [[ ! -e "$p" ]]; then
    fail "precondition: $p does not exist on the host"
  elif grep -qxF "absent $p" <<<"$out"; then
    pass "not visible: $p"
  else
    fail "visible in the sandbox: $p"
  fi
done
host_entries="$(find "$real_home" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)"
out="$(in_sandbox sh -c 'find "$1" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l' sh "$real_home")" || true
if ((host_entries == 0)); then
  printf 'NOTE: %s is empty on this host; listing not checked\n' "$real_home"
elif ((out <= 1)); then
  pass "the real home shows at most the one path toward the worktree ($out of $host_entries entries)"
else
  fail "the real home lists $out entries in the sandbox (host: $host_entries)"
fi

# --- 3. environment and the one credential ------------------------------------------
out="$(in_sandbox sh -c 'env; echo "---"; cat "$HOME/.nodecode/secrets/providers.canary.api_key"' sh)" || true
envpart="${out%%---*}"
credpart="${out#*---}"
if grep -qF "$canary_value" <<<"$envpart"; then
  fail "the caller's environment reached the sandbox"
else
  pass "caller environment cleared (canary variable absent)"
fi
extra="$(grep -v -E '^(HOME|NODECODE_HOME|PATH|TERM|NO_COLOR|LANG|TMPDIR|USER|PWD|SHLVL|_)=' <<<"$envpart" | grep -v '^$' || true)"
[[ -z "$extra" ]] && pass "sandbox environment is the fixed allow-list" || fail "unexpected variables: $extra"
grep -qxF "$canary_value" <<<"$credpart" && pass "the one credential is readable at the nodecode secret-file path" \
  || fail "credential not at the secret-file path"
out="$(in_sandbox sh -c 'cat "$HOME/.nodecode/config.jsonc"' sh)" || true
grep -q '"mode": "off"' <<<"$out" && pass "worker nodecode config sets update.mode off" \
  || fail "worker config does not turn self-update off: $out"
out="$(in_sandbox sh -c 'f="$HOME/.nodecode/secrets/providers.canary.api_key"; if ( : >>"$f" ) 2>/dev/null; then echo writable; else echo readonly; fi' sh)" || true
[[ "$out" == "readonly" ]] && pass "credential file is read-only" || fail "credential file is writable"
if grep -rqF "$canary_value" -- "$wt" 2>/dev/null; then
  fail "the credential value appears in the worktree"
else
  pass "the credential value appears nowhere in the worktree"
fi

# --- 3b. privileges, descriptors, terminal, /dev, core dumps -----------------------
fd_target="$work/host-fd-target"
: >"$fd_target"
harden_probe='( echo leak >&9 ) 2>/dev/null && echo fd9-writable || echo fd9-closed
grep -E "^(CapEff|NoNewPrivs):" /proc/self/status | tr -d "\t"
unshare -Ur true >/dev/null 2>&1 && echo userns-allowed || echo userns-refused
(exec 3</dev/tty) 2>/dev/null && echo tty-open || echo tty-none
echo "tty_nr=$(cut -d" " -f7 /proc/self/stat)"
( : >/dev/meshloop-probe ) 2>/dev/null && echo dev-writable || echo dev-readonly
( : >/dev/null ) 2>/dev/null && echo devnull-ok || echo devnull-broken
echo "shm_bytes=$(( $(stat -f -c %b /dev/shm) * $(stat -f -c %S /dev/shm) ))"
echo "core_soft=$(ulimit -c) core_hard=$(ulimit -Hc)"'
out="$(in_sandbox sh -c "$harden_probe" sh 9>>"$fd_target")" || true
check_line() { # $1 expected line, $2 description
  grep -qxF -- "$1" <<<"$out" && pass "$2" || fail "$2: expected '$1' in: $(tr '\n' ' ' <<<"$out")"
}
check_line fd9-closed "a descriptor the caller left open (fd 9) is closed in the sandbox"
[[ ! -s "$fd_target" ]] && pass "nothing reached the host file behind fd 9" || fail "a sandbox write reached the host through fd 9"
check_line "CapEff:0000000000000000" "no effective capabilities"
check_line "NoNewPrivs:1" "no_new_privs is set"
check_line userns-refused "unshare -Ur fails (no nested user namespaces)"
check_line tty-none "no controlling terminal (/dev/tty cannot be opened)"
check_line tty_nr=0 "the process has no tty (tty_nr 0)"
check_line dev-readonly "/dev is read-only"
check_line devnull-ok "/dev/null still works"
check_line shm_bytes=1073741824 "/dev/shm is a tmpfs capped at --tmpfs-size (1 GiB)"
check_line "core_soft=0 core_hard=0" "core dumps are off (soft and hard limit 0)"

# --- 3c. refusals: release folder and credential file ---------------------------------
refused() { # $1 description, $2 expected message fragment, rest: command (run in $wt)
  local desc="$1" frag="$2" rc=0 msg
  shift 2
  msg="$(cd "$wt" && HOME="$fake_home" "$@" 2>&1 >/dev/null)" || rc=$?
  if ((rc == 125)) && grep -qF -- "$frag" <<<"$msg"; then
    pass "refused: $desc"
  else
    fail "not refused ($rc): $desc: $msg"
  fi
}
fake_release() { # $1 dir: a fake release folder (binary + src/)
  mkdir -p "$1/src"
  printf '#!/bin/sh\necho fake\n' >"$1/nodecode"
  chmod +x "$1/nodecode"
}
fake_release "$fake_home"
refused "a release folder that is the operator's home" "operator's home" \
  "$wrapper" --nodecode "$fake_home/nodecode" --version
fake_release "$fake_home/.nodecode/rel"
refused "a release folder inside ~/.nodecode" "nodecode home" \
  "$wrapper" --nodecode "$fake_home/.nodecode/rel/nodecode" --version
fake_release "$wt/rel"
refused "a release folder inside the worktree" "inside the worktree" \
  "$wrapper" --nodecode "$wt/rel/nodecode" -- --help
rm -rf "$wt/rel" "$fake_home/nodecode" "$fake_home/src"
(umask 077 && printf '%s\n' "$canary_value" >"$wt/key-in-worktree" && printf '%s\n' "$canary_value" >"$repo/key-in-repo")
refused "a credential file inside the worktree" "inside the worktree" \
  "$wrapper" --credential-provider canary --credential-file "$wt/key-in-worktree" --run-command -- true
refused "a credential file inside the repository" "inside the repository" \
  "$wrapper" --credential-provider canary --credential-file "$repo/key-in-repo" --run-command -- true
rm -f "$wt/key-in-worktree" "$repo/key-in-repo"
cp "$cred_file" "$work/loose.key" && chmod 644 "$work/loose.key"
refused "a group- or world-readable credential file" "readable by group or others" \
  "$wrapper" --credential-provider canary --credential-file "$work/loose.key" --run-command -- true

# --- 3d. the turn driver scrubs the worker's key ---------------------------------------
# A fake nodecode release whose `rpc` prints the bound key in a command result, an
# assistant message and on stderr, plus malformed frames, then ends the turn.
if [[ -x /usr/bin/python3 ]]; then
  fake_release "$work/fake-release"
  cat >"$work/fake-release/nodecode" <<'FAKE'
#!/bin/sh
key=$(head -n 1 "$HOME/.nodecode/secrets/providers.canary.api_key")
read -r line
case "$line" in *'"provider": "canary"'*'"model": "fake-model"'*) ok=yes ;; *) ok=no ;; esac
echo "stderr says $key" >&2
printf '{"sequence":0,"payload":{"type":"gateway_turn_command_result","result":{"command_id":"meshloop-turn-1","status":"accepted","message":"ok %s"}}}\n' "$key"
printf '[1,2]\n"text"\n{"payload":"x"}\n{"payload":{"type":"gateway_turn_command_result","result":7}}\nnot json\n'
printf '{"sequence":1,"payload":{"type":"session_stream_event","frame":{"event":{"type":"turn.assistant_message_completed","payload":{"message":{"content":"provider-split=%s the key is %s"}}}}}}\n' "$ok" "$key"
printf '{"sequence":2,"payload":{"type":"session_stream_event","frame":{"event":{"type":"turn.completed","payload":{}}}}}\n'
while read -r l; do :; done
FAKE
  chmod +x "$work/fake-release/nodecode"
  printf 'say hi\n' >"$wt/prompt.txt"
  rc=0
  (cd "$wt" && HOME="$fake_home" "$wrapper" --nodecode "$work/fake-release/nodecode" \
    --credential-provider canary --credential-file "$cred_file" \
    --model canary/fake-model --prompt-file "$wt/prompt.txt" --raw-log "$wt/raw.jsonl") \
    >"$work/turn.out" 2>"$work/turn.err" || rc=$?
  if ((rc == 0)) && grep -qx 'nodecode-turn: turn.completed' "$work/turn.out"; then
    pass "a turn through the wrapper and driver completes (malformed frames ignored)"
  else
    fail "fake turn did not complete ($rc): $(cat "$work/turn.out" "$work/turn.err")"
  fi
  grep -q 'provider-split=yes' "$work/turn.out" && pass "start_turn names provider and model separately" \
    || fail "start_turn did not split PROVIDER/MODEL: $(cat "$work/turn.out")"
  if grep -qF "$canary_value" "$work/turn.out" "$work/turn.err" "$wt/raw.jsonl"; then
    fail "the key value reached stdout, stderr or the raw log"
  elif grep -q 'the key is \[redacted\]' "$work/turn.out" && grep -q 'stderr says \[redacted\]' "$work/turn.err" \
    && grep -q 'ok \[redacted\]' "$wt/raw.jsonl"; then
    pass "the key value is scrubbed from stdout, stderr and the raw log"
  else
    fail "scrubbing not shown: $(cat "$work/turn.out" "$work/turn.err")"
  fi
  rm -f "$wt/prompt.txt" "$wt/raw.jsonl"
else
  printf 'NOTE: /usr/bin/python3 is absent; driver checks not run\n'
fi

# --- 4. process tree: killing the owned process group leaves nothing ---------------
# meshloop spawns the wrapper as a process-group leader and kills the group with SIGKILL
# (ADR 0025). Start the same way, with children that detach into their own session.
tree_probe='setsid sh -c "sleep 1000; :" "$0-detached" </dev/null >/dev/null 2>&1 &
sh -c "sleep 1000; :" "$0-child" &
echo started; wait'
(cd "$wt" && HOME="$fake_home" exec setsid "$wrapper" --run-command -- sh -c "$tree_probe" "$marker") >"$work/tree.out" 2>&1 &
launcher=$!
for _ in $(seq 1 50); do
  grep -q started "$work/tree.out" 2>/dev/null && break
  sleep 0.1
done
sandbox_pid="$(pgrep -f -o -- "bwrap .*$marker" || true)"
ns=""
if [[ -n "$sandbox_pid" ]]; then
  for pid in $(pgrep -f -- "$marker-child" || true); do
    ns="$(readlink "/proc/$pid/ns/pid" 2>/dev/null || true)"
    [[ -n "$ns" ]] && break
  done
fi
before="$(pgrep -f -- "$marker" | wc -l)"
if [[ -z "$sandbox_pid" || -z "$ns" || "$before" -lt 3 ]]; then
  fail "process-tree probe did not start (bwrap=$sandbox_pid ns=$ns processes=$before)"
else
  kill -KILL -- "-$sandbox_pid"
  survivors=1
  for _ in $(seq 1 50); do
    left_marker="$(pgrep -f -- "$marker" || true)"
    left_ns=""
    for p in /proc/[0-9]*; do
      [[ "$(readlink "$p/ns/pid" 2>/dev/null || true)" == "$ns" ]] && left_ns+="${p#/proc/} "
    done
    if [[ -z "$left_marker" && -z "$left_ns" ]]; then survivors=0; break; fi
    sleep 0.1
  done
  if ((survivors)); then
    fail "processes survived the group kill: marker=[$left_marker] ns=[$left_ns]"
  else
    pass "group SIGKILL of the wrapper leaves no process (marker and pid namespace empty; $before before)"
  fi
fi
wait "$launcher" 2>/dev/null || true
sandbox_pid=""

# --- 5. owner death: SIGKILL of bwrap's parent also ends the sandbox ----------------
(cd "$wt" && HOME="$fake_home" exec "$wrapper" --run-command -- sh -c 'sh -c "sleep 1000; :" "$0-orphan" & echo started; wait' "$marker") >"$work/orphan.out" 2>&1 &
owner=$!
for _ in $(seq 1 50); do
  grep -q started "$work/orphan.out" 2>/dev/null && break
  sleep 0.1
done
# The background subshell exec'd the wrapper, which exec'd bwrap: $owner is bwrap.
kill -KILL "$owner" 2>/dev/null || true
gone=0
for _ in $(seq 1 50); do
  if ! pgrep -f -- "$marker-orphan" >/dev/null; then gone=1; break; fi
  sleep 0.1
done
((gone)) && pass "SIGKILL of bwrap leaves no sandboxed process (--die-with-parent, pid namespace)" \
  || fail "sandboxed process survived bwrap's death"
wait "$owner" 2>/dev/null || true

parent_probe='sh -c "sleep 1000; :" "$0-parent" & echo started; wait'
bash -c 'cd "$1" && HOME="$2" "$3" --run-command -- sh -c "$4" "$5"; :' owner "$wt" "$fake_home" "$wrapper" "$parent_probe" "$marker" >"$work/parent.out" 2>&1 &
owner=$!
for _ in $(seq 1 50); do
  grep -q started "$work/parent.out" 2>/dev/null && break
  sleep 0.1
done
kill -KILL "$owner" 2>/dev/null || true
gone=0
for _ in $(seq 1 50); do
  if ! pgrep -f -- "$marker" >/dev/null; then gone=1; break; fi
  sleep 0.1
done
((gone)) && pass "SIGKILL of the wrapper's parent ends bwrap and the sandbox (--die-with-parent)" \
  || fail "sandbox survived its parent: $(pgrep -af -- "$marker" || true)"
wait "$owner" 2>/dev/null || true

# --- 6. the operator's nodecode homes are never mounted --------------------------
# mountinfo's fourth field is the host path a bind mount shows; the only host paths
# under a home are the worktree, the git dirs and the credential file.
out="$(in_sandbox sh -c 'cat /proc/self/mountinfo' sh)" || true
if grep -qF -e "$real_home/.nodecode" -e "$fake_home/.nodecode" -e "$fake_home " <<<"$out"; then
  fail "an operator home or nodecode home is mounted in the sandbox"
else
  pass "no operator home or nodecode home is mounted in the sandbox"
fi

if ((failures)); then
  printf 'containment-test: %d check(s) FAILED\n' "$failures"
  exit 1
fi
printf 'containment-test: all checks passed\n'
