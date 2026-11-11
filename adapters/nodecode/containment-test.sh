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
# namespaces). Exit 1: a containment check failed.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
wrapper="$here/meshloop-nodecode-sandbox"
repo_root="$(cd -- "$here/../.." && pwd -P)"

skip() {
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
git_q -C "$repo" -c user.email=t@t -c user.name=t commit -q --allow-empty -m seed
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

# --- 2. visibility: the operator's homes and secrets are not mounted -----------------
vis_probe='for p in "$@"; do if [ -e "$p" ]; then echo "visible $p"; else echo "absent $p"; fi; done'
hidden=("$fake_home/.nodecode" "$fake_home/.ssh" "$real_home/.nodecode" "$real_home/.ssh" "$real_home/.config" "$repo/README.md")
out="$(in_sandbox sh -c "$vis_probe" sh "${hidden[@]}")" || true
for p in "${hidden[@]}"; do
  grep -qxF "absent $p" <<<"$out" && pass "not visible: $p" || fail "visible in the sandbox: $p"
done

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
