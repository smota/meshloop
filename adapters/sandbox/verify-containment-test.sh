#!/usr/bin/env bash
# Deterministic containment test for meshloop-verify-sandbox (issue #181).
#
# Needs bash, git and bubblewrap with unprivileged user namespaces; the network checks
# also need /usr/bin/python3 (for a listener on the host's 127.0.0.1) and are skipped,
# with a line, without it. Needs no network beyond loopback and no toolchain.
#
#   adapters/sandbox/verify-containment-test.sh [BASE_DIR]
#
# BASE_DIR must be on ext4, xfs or btrfs (default: <repo>/.agent-runs). The test makes a
# disposable repository with a linked worktree under it and removes it on exit.
# Exit 0: passed, or skipped with a "SKIP:" line (not Linux, no bwrap, no user
# namespaces). Exit 1: a containment check failed, or a skip while
# MESHLOOP_REQUIRE_CONTAINMENT=1 (CI sets it where a probe shows bwrap works).
#
# Every probe is a script the "worker" wrote into the worktree, run as the verify step
# through the wrapper, exactly as meshloop runs `[verify] runner` + verify_command.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
wrapper="$here/meshloop-verify-sandbox"
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
  --unshare-cgroup-try --unshare-net --die-with-parent --new-session --ro-bind / / true 2>&1)"; then
  skip "bwrap cannot build the sandbox here (user namespaces or bubblewrap >= 0.8 missing): $probe_err"
fi

base="${1:-$repo_root/.agent-runs}"
mkdir -p -- "$base"
case "$(stat -f -c %T -- "$base")" in
  ext2/ext3 | xfs | btrfs) ;;
  *) skip "BASE_DIR $base is not on ext4, xfs or btrfs, which the wrapper requires" ;;
esac
work="$(mktemp -d "$base/verify-containment-test.XXXXXXXX")"
marker="meshloop-verify-containment-$(basename -- "$work")"
real_home="$(getent passwd "$(id -u)" | cut -d: -f6)"
probe_name=".meshloop-verify-probe-$$"
failures=0
sandbox_pid=""
listener_pid=""

cleanup() {
  if [[ -n "$sandbox_pid" ]]; then kill -KILL -- "-$sandbox_pid" 2>/dev/null || true; fi
  if [[ -n "$listener_pid" ]]; then kill -KILL "$listener_pid" 2>/dev/null || true; fi
  pkill -KILL -f -- "$marker" 2>/dev/null || true
  # A probe that escaped would have left a file; remove it either way.
  rm -f -- "$real_home/$probe_name" "/tmp/$probe_name" 2>/dev/null || true
  rm -rf -- "$work"
}
trap cleanup EXIT

pass() { printf 'PASS: %s\n' "$*"; }
fail() {
  printf 'FAIL: %s\n' "$*"
  failures=$((failures + 1))
}

# --- layout: a repository with a linked worktree inside it (meshloop's default
# layout), a fake operator home with a secret, an ssh key, a nodecode home and a fake
# toolchain whose folder holds a Cargo-style credentials.toml.
repo="$work/repo"
fake_home="$work/home"
tool_home="$fake_home/.cargo"
git_q() { env -i PATH=/usr/bin:/bin HOME="$fake_home" GIT_CONFIG_NOSYSTEM=1 git "$@" >/dev/null 2>&1; }
mkdir -p "$repo" "$fake_home/.ssh" "$fake_home/.nodecode/rel" "$tool_home/bin"
secret_value="secret-$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')"
printf '%s\n' "$secret_value" >"$fake_home/.secret"
printf '%s\n' "$secret_value" >"$fake_home/.ssh/id_test"
printf 'token = "%s"\n' "$secret_value" >"$tool_home/credentials.toml"
printf '#!/bin/sh\necho fake-toolchain-ok\n' >"$tool_home/bin/faketool"
chmod +x "$tool_home/bin/faketool"
git_q -C "$repo" init -q
printf 'seed\n' >"$repo/README.md"
git_q -C "$repo" add README.md
git_q -C "$repo" -c user.email=t@t -c user.name=t commit -q -m seed
git_q -C "$repo" worktree add -q "$repo/.meshloop/worktrees/w1" -b w1
wt="$repo/.meshloop/worktrees/w1"
[[ -f "$wt/.git" ]] || { echo "FAIL: could not create a linked worktree"; exit 1; }

# The worker's check script: written into the worktree, run by the verify step.
worker_script="$wt/worker-check.sh"

# Runs the worker-written script as the verify step: the wrapper with the fake home as
# HOME, the caller's environment plus a canary variable, the worktree as working
# directory, extra wrapper options first. Echoes the exit code last as rc=N.
verify_step() { # [WRAPPER-OPTIONS...] -- SCRIPT-ARGS...
  local opts=()
  while (($#)) && [[ "$1" != "--" ]]; do opts+=("$1"); shift; done
  shift
  local rc=0
  (cd "$wt" && HOME="$fake_home" MESHLOOP_CANARY_ENV="$secret_value" \
    "$wrapper" --worktree "$wt" --ro-path "$tool_home" --path "$tool_home/bin" \
    "${opts[@]}" -- sh worker-check.sh "$@") || rc=$?
  printf 'rc=%s\n' "$rc"
}

check_line() { # $1 expected line, $2 description (reads $out)
  grep -qxF -- "$1" <<<"$out" && pass "$2" || fail "$2: expected '$1' in: $(tr '\n' ' ' <<<"$out")"
}

# --- 1. writes outside the worktree fail -------------------------------------------
cat >"$worker_script" <<'EOF'
for t in "$@"; do
  if ( : >"$t" ) 2>/dev/null; then echo "wrote $t"; else echo "refused $t"; fi
done
EOF
targets=(
  "$wt/$probe_name"             # the worktree: must succeed
  "$repo/$probe_name"           # the repository root above the worktree
  "$repo/.git/$probe_name"      # the git common dir (bound read-only)
  "$fake_home/$probe_name"      # the operator's home (the HOME given)
  "$real_home/$probe_name"      # the real operator home
  "$tool_home/$probe_name"      # a --ro-path toolchain folder
  "/tmp/$probe_name"            # /tmp: the sandbox's own tmpfs
  "/usr/$probe_name"
  "/etc/$probe_name"
  "/$probe_name"
)
out="$(verify_step -- "${targets[@]}")"
if [[ -f "$wt/$probe_name" ]] && grep -qxF "wrote $wt/$probe_name" <<<"$out"; then
  pass "the check writes inside its worktree"
else
  fail "the check could not write inside its worktree: $out"
fi
for t in "${targets[@]:1}"; do
  if [[ "$t" == "/tmp/$probe_name" ]]; then
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
rm -f -- "$wt/$probe_name"

# The worktree's .git file is read-only: the check cannot redirect meshloop's later
# host-side git commands to a gitdir (and config) of its own.
cat >"$worker_script" <<'EOF'
{ printf "gitdir: /tmp/evil\n" >.git; } 2>/dev/null && echo rewritten || echo refused-write
rm -f .git 2>/dev/null && echo removed || echo refused-remove
git rev-parse --is-inside-work-tree 2>/dev/null || echo git-unusable
EOF
before_git="$(cat "$wt/.git")"
out="$(verify_step --)"
if [[ "$(cat "$wt/.git" 2>/dev/null)" == "$before_git" ]] \
  && grep -qx refused-write <<<"$out" && grep -qx refused-remove <<<"$out"; then
  pass "the worktree's .git file cannot be rewritten or removed"
else
  fail "the worktree's .git file is writable by the check: $out"
fi
check_line true "git works read-only inside the worktree"

# --- 2. reads: secrets are not mounted, /usr and the toolchain are -----------------
cat >"$worker_script" <<'EOF'
for p in "$@"; do if [ -e "$p" ]; then echo "visible $p"; else echo "absent $p"; fi; done
echo "---"
cat "$HOME/.secret" 2>/dev/null || echo no-home-secret
echo "creds=[$(cat "$TOOL_HOME/credentials.toml" 2>&1)]"
faketool 2>&1 || echo faketool-missing
[ -r /usr/bin/env ] && ls /usr/bin >/dev/null && echo usr-readable || echo usr-unreadable
echo "---"
env
EOF
hidden=("$fake_home/.secret" "$fake_home/.ssh" "$fake_home/.nodecode" "$repo/README.md")
for p in "$real_home/.ssh" "$real_home/.config" "$real_home/.nodecode" "$real_home/.gitconfig"; do
  if [[ -e "$p" ]]; then hidden+=("$p"); else printf 'NOTE: %s is absent on this host; not checked\n' "$p"; fi
done
out="$(verify_step --env "TOOL_HOME=$tool_home" -- "${hidden[@]}")"
vispart="${out%%---*}"
rest="${out#*---}"
readpart="${rest%%---*}"
envpart="${rest#*---}"
for p in "${hidden[@]}"; do
  if [[ ! -e "$p" ]]; then
    fail "precondition: $p does not exist on the host"
  elif grep -qxF "absent $p" <<<"$vispart"; then
    pass "not visible: $p"
  else
    fail "visible in the sandbox: $p"
  fi
done
grep -qx no-home-secret <<<"$readpart" && pass "the fake home's secret cannot be read" \
  || fail "the fake home's secret was read: $readpart"
grep -qxF "creds=[]" <<<"$readpart" && pass "a credentials.toml inside a --ro-path reads as empty" \
  || fail "credentials.toml not masked: $readpart"
grep -qx fake-toolchain-ok <<<"$readpart" && pass "a --ro-path toolchain runs from --path" \
  || fail "the --ro-path toolchain did not run: $readpart"
grep -qx usr-readable <<<"$readpart" && pass "/usr is readable" || fail "/usr not readable: $readpart"
if grep -qF "$secret_value" <<<"$out"; then
  fail "the secret value reached the check's output"
else
  pass "the secret value appears nowhere in the check's output (canary variable cleared)"
fi
extra="$(grep -v -E '^(HOME|PATH|TERM|NO_COLOR|LANG|TMPDIR|USER|TOOL_HOME|PWD|OLDPWD|SHLVL|_)=' <<<"$envpart" | grep -v -e '^$' -e '^rc=[0-9]*$' || true)"
[[ -z "$extra" ]] && pass "sandbox environment is the fixed allow-list plus --env" || fail "unexpected variables: $extra"
grep -qxF "PATH=$tool_home/bin:/usr/local/bin:/usr/bin:/bin" <<<"$envpart" && pass "--path goes in front of PATH" \
  || fail "PATH not as expected: $(grep '^PATH=' <<<"$envpart")"

# --- 3. network: off by default, on only with --allow-network -------------------------
if [[ -x /usr/bin/python3 ]]; then
  port_file="$work/listener.port"
  hit_file="$work/listener.hit"
  /usr/bin/python3 -I -c '
import socket, sys
s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(8)
open(sys.argv[1] + ".tmp", "w").write(str(s.getsockname()[1]))
import os; os.rename(sys.argv[1] + ".tmp", sys.argv[1])
s.settimeout(60)
while True:
    try:
        c, _ = s.accept()
    except OSError:
        break
    open(sys.argv[2], "a").write("hit\n"); c.close()
' "$port_file" "$hit_file" &
  listener_pid=$!
  for _ in $(seq 1 50); do [[ -s "$port_file" ]] && break; sleep 0.1; done
  port="$(cat "$port_file" 2>/dev/null || true)"
  if [[ -z "$port" ]]; then
    fail "the host listener did not start"
  else
    cat >"$worker_script" <<'EOF'
bash -c 'exec 3<>"/dev/tcp/127.0.0.1/$1"' sh "$1" 2>/dev/null && echo net-open || echo net-closed
EOF
    out="$(verify_step -- "$port")"
    check_line net-closed "a connection to a host 127.0.0.1 listener fails by default"
    [[ ! -s "$hit_file" ]] && pass "the host listener saw no connection" || fail "the host listener was reached"
    out="$(verify_step --allow-network -- "$port")"
    check_line net-open "with --allow-network the same connection succeeds (control)"
  fi
  kill -KILL "$listener_pid" 2>/dev/null || true
  wait "$listener_pid" 2>/dev/null || true
  listener_pid=""
else
  printf 'NOTE: /usr/bin/python3 is absent; network checks not run\n'
fi

# --- 4. inherited descriptors, privileges, terminal, core dumps ---------------------
fd_target="$work/host-fd-target"
: >"$fd_target"
cat >"$worker_script" <<'EOF'
( echo leak >&9 ) 2>/dev/null && echo fd9-writable || echo fd9-closed
grep -E "^(CapEff|NoNewPrivs):" /proc/self/status | tr -d "\t"
unshare -Ur true >/dev/null 2>&1 && echo userns-allowed || echo userns-refused
(exec 3</dev/tty) 2>/dev/null && echo tty-open || echo tty-none
( : >/dev/meshloop-probe ) 2>/dev/null && echo dev-writable || echo dev-readonly
echo "core_soft=$(ulimit -c) core_hard=$(ulimit -Hc)"
EOF
out="$(cd "$wt" && HOME="$fake_home" "$wrapper" --worktree "$wt" -- sh worker-check.sh 9>>"$fd_target")" || true
check_line fd9-closed "a descriptor the caller left open (fd 9) is closed in the sandbox"
[[ ! -s "$fd_target" ]] && pass "nothing reached the host file behind fd 9" || fail "a sandbox write reached the host through fd 9"
check_line "CapEff:0000000000000000" "no effective capabilities"
check_line "NoNewPrivs:1" "no_new_privs is set"
check_line userns-refused "unshare -Ur fails (no nested user namespaces)"
check_line tty-none "no controlling terminal"
check_line dev-readonly "/dev is read-only"
check_line "core_soft=0 core_hard=0" "core dumps are off"

# --- 5. exit codes propagate: verification semantics are unchanged -----------------
printf 'exit "$1"\n' >"$worker_script"
for code in 0 1 7; do
  out="$(verify_step -- "$code")"
  check_line "rc=$code" "the check's exit code $code is the wrapper's exit code"
done
rm -f -- "$worker_script"

# --- 6. refusals (exit 125, no sandbox run) ------------------------------------------
refused() { # $1 description, $2 expected message fragment, $3 dir, rest: wrapper args
  local desc="$1" frag="$2" dir="$3" rc=0 msg
  shift 3
  msg="$(cd "$dir" && HOME="$fake_home" "$wrapper" "$@" 2>&1 >/dev/null)" || rc=$?
  if ((rc == 125)) && grep -qF -- "$frag" <<<"$msg"; then
    pass "refused: $desc"
  else
    fail "not refused ($rc): $desc: $msg"
  fi
}
refused "no --worktree" "--worktree is required" "$wt" -- true
refused "a relative --worktree" "absolute" "$wt" --worktree . -- true
refused "a working directory other than --worktree" "not the --worktree" "$repo" --worktree "$wt" -- true
refused "the main repository (not a linked worktree)" "not a linked git worktree" "$repo" --worktree "$repo" -- true
refused "--ro-path /" "refusing /" "$wt" --worktree "$wt" --ro-path / -- true
refused "--ro-path of the operator's home" "operator's home" "$wt" --worktree "$wt" --ro-path "$fake_home" -- true
refused "--ro-path of ~/.ssh" "secret store" "$wt" --worktree "$wt" --ro-path "$fake_home/.ssh" -- true
refused "--ro-path inside ~/.nodecode" "nodecode home" "$wt" --worktree "$wt" --ro-path "$fake_home/.nodecode/rel" -- true
refused "--ro-path of the repository (contains the worktree)" "contains the worktree" "$wt" --worktree "$wt" --ro-path "$repo" -- true
refused "--ro-path inside the worktree" "inside the worktree" "$wt" --worktree "$wt" --ro-path "$wt/README.md" -- true
refused "--ro-path of the git directory" "git directory" "$wt" --worktree "$wt" --ro-path "$repo/.git" -- true
refused "--env HOME" "must not set HOME" "$wt" --worktree "$wt" --env HOME=/x -- true
refused "--env with a credential-like name" "looks like a credential" "$wt" --worktree "$wt" --env GH_TOKEN=x -- true
refused "--path outside /usr and every --ro-path" "under /usr or a --ro-path" "$wt" --worktree "$wt" --path /opt/bin -- true
refused "no program" "no program" "$wt" --worktree "$wt" --

# Secret stores are refused when a --ro-path is, is inside, or contains one.
mkdir -p "$fake_home/.local/share/keyrings" "$fake_home/.claude" "$fake_home/.local/share/tool"
printf '%s\n' "$secret_value" >"$fake_home/.local/share/keyrings/login.keyring"
refused "--ro-path ~/.local (contains ~/.local/share/keyrings)" "contains a secret store" "$wt" --worktree "$wt" --ro-path "$fake_home/.local" -- true
refused "--ro-path ~/.local/share (contains the keyrings)" "contains a secret store" "$wt" --worktree "$wt" --ro-path "$fake_home/.local/share" -- true
refused "--ro-path ~/.claude" "inside a secret store" "$wt" --worktree "$wt" --ro-path "$fake_home/.claude" -- true
# Host socket and state folders are refused.
refused "--ro-path /tmp" "/tmp" "$wt" --worktree "$wt" --ro-path /tmp -- true
refused "--ro-path /run" "/run" "$wt" --worktree "$wt" --ro-path /run -- true
refused "--ro-path /proc/self" "/proc" "$wt" --worktree "$wt" --ro-path /proc/self -- true
refused "--ro-path /var (contains /var/run)" "/var/run" "$wt" --worktree "$wt" --ro-path /var -- true

# --print-bwrap prints and never exits 0, so a runner prefix that kept it cannot make
# every check pass.
rc=0
(cd "$wt" && HOME="$fake_home" "$wrapper" --worktree "$wt" --print-bwrap -- true) >"$work/print.out" 2>&1 || rc=$?
((rc == 2)) && grep -q '^bwrap ' "$work/print.out" && pass "--print-bwrap prints the argv and exits 2" \
  || fail "--print-bwrap exited $rc: $(cat "$work/print.out")"

# A credentials.toml under --env CARGO_HOME (a sub-folder of a --ro-path) reads as empty.
mkdir -p "$fake_home/.local/share/tool/cargo"
printf 'token = "%s"\n' "$secret_value" >"$fake_home/.local/share/tool/cargo/credentials.toml"
printf 'cat "$CARGO_HOME/credentials.toml" 2>&1; echo "[end]"\n' >"$worker_script"
out="$(verify_step --ro-path "$fake_home/.local/share/tool" --env "CARGO_HOME=$fake_home/.local/share/tool/cargo" --)"
if grep -qF "$secret_value" <<<"$out"; then
  fail "credentials.toml under CARGO_HOME was readable: $out"
else
  check_line "[end]" "a credentials.toml under --env CARGO_HOME reads as empty"
fi
rm -f -- "$worker_script"

# The documented shape: --ro-path ~/.cargo plus --env CARGO_HOME=~/.cargo with a
# credentials.toml present (masked once, not twice), and an explicit duplicate --mask.
printf 'cat "$CARGO_HOME/credentials.toml" 2>&1; echo "[end]"\n' >"$worker_script"
out="$(verify_step --env "CARGO_HOME=$tool_home" -- )"
if grep -qF "$secret_value" <<<"$out"; then
  fail "credentials.toml readable in the documented CARGO_HOME shape: $out"
else
  check_line "[end]" "documented shape (--ro-path + --env CARGO_HOME, credentials.toml present) runs, file empty"
  check_line "rc=0" "documented shape exits 0"
fi
out="$(verify_step --env "CARGO_HOME=$tool_home" --mask "$tool_home/credentials.toml" --mask "$tool_home/credentials.toml" --)"
check_line "rc=0" "an explicit duplicate --mask of the same file still runs"
grep -qF "$secret_value" <<<"$out" && fail "credentials.toml readable with duplicate masks: $out" \
  || pass "credentials.toml stays empty with duplicate masks"
rm -f -- "$worker_script"

# A secret store that is itself a symlink (~/.aws -> vault/aws): a --ro-path containing
# its target is refused.
mkdir -p "$fake_home/vault/aws"
printf '%s\n' "$secret_value" >"$fake_home/vault/aws/credentials"
ln -s vault/aws "$fake_home/.aws"
refused "--ro-path containing the target of a symlinked ~/.aws" "contains a secret store" "$wt" --worktree "$wt" --ro-path "$fake_home/vault" -- true
refused "--ro-path at the target of a symlinked ~/.aws" "secret store" "$wt" --worktree "$wt" --ro-path "$fake_home/vault/aws" -- true

# --- 6b. git metadata: credentials in git config, forged `.git` files ----------------
# Credentials in the repository's git config refuse --git-metadata ro.
git_q -C "$repo" config http.https://example.invalid/.extraheader "AUTHORIZATION: basic eHg6eXk="
refused "an http extraheader in git config" "--git-metadata none" "$wt" --worktree "$wt" -- true
rc=0
(cd "$wt" && HOME="$fake_home" "$wrapper" --worktree "$wt" --git-metadata none -- true) || rc=$?
((rc == 0)) && pass "--git-metadata none runs despite credentials in git config" || fail "--git-metadata none exited $rc"
git_q -C "$repo" config --unset-all http.https://example.invalid/.extraheader
git_q -C "$repo" remote add origin "https://user:tok@example.invalid/r.git"
refused "user:token@ in a remote URL" "--git-metadata none" "$wt" --worktree "$wt" -- true
git_q -C "$repo" remote remove origin
# An ssh://git@host remote carries a user name, not a credential: not refused.
git_q -C "$repo" remote add origin "ssh://git@example.invalid/r.git"
rc=0
(cd "$wt" && HOME="$fake_home" "$wrapper" --worktree "$wt" -- true) || rc=$?
((rc == 0)) && pass "an ssh://git@host remote is not refused" || fail "an ssh://git@ remote was refused ($rc)"
git_q -C "$repo" remote remove origin
# A credential in a file included (relative include.path) from the git config.
printf '[http "https://example.invalid/"]\n\textraheader = AUTHORIZATION: basic eHg6eXk=\n' >"$repo/.git/creds.inc"
git_q -C "$repo" config include.path creds.inc
refused "an extraheader in a relative include.path file" "--git-metadata none" "$wt" --worktree "$wt" -- true
git_q -C "$repo" config --unset include.path
rm -f -- "$repo/.git/creds.inc"

# A forged `.git` whose admin dir is not <common>/worktrees/<name> binds no git dir:
# its commondir pointing at a fake ~/.ssh exposes nothing.
gitfile_orig="$(cat "$wt/.git")"
forged="$work/forged-admin"
mkdir -p "$forged"
printf '%s\n' "$wt/.git" >"$forged/gitdir"
printf '%s\n' "$fake_home/.ssh" >"$forged/commondir"
printf 'gitdir: %s\n' "$forged" >"$wt/.git"
printf 'for p in "$@"; do [ -e "$p" ] && echo "visible $p" || echo "absent $p"; done\n' >"$worker_script"
out="$(verify_step -- "$fake_home/.ssh" "$fake_home/.ssh/id_test" "$forged")"
check_line "absent $fake_home/.ssh/id_test" "a forged .git with commondir at ~/.ssh binds nothing (key absent)"
check_line "absent $forged" "a forged admin dir outside <common>/worktrees is not bound"
check_line "rc=0" "the check still runs with no git dir bound (fail closed, not refused)"
# An admin dir that does follow the layout, but inside a secret store, is refused.
mkdir -p "$fake_home/.ssh/worktrees/w1"
printf '%s\n' "$wt/.git" >"$fake_home/.ssh/worktrees/w1/gitdir"
printf '../..\n' >"$fake_home/.ssh/worktrees/w1/commondir"
printf 'gitdir: %s\n' "$fake_home/.ssh/worktrees/w1" >"$wt/.git"
refused "git dirs inside a secret store" "secret store" "$wt" --worktree "$wt" -- true
printf '%s\n' "$gitfile_orig" >"$wt/.git"
rm -rf -- "$fake_home/.ssh/worktrees" "$forged" "$worker_script"

# --- 7. process tree: killing the owned process group leaves nothing ---------------
# meshloop spawns the runner as a process-group leader and, on timeout or cancel, kills
# the group (ADR 0025). Start the same way, with a child that detaches into its own
# session.
cat >"$worker_script" <<'EOF'
setsid sh -c "sleep 1000; :" "$1-detached" </dev/null >/dev/null 2>&1 &
sh -c "sleep 1000; :" "$1-child" &
echo started; wait
EOF
(cd "$wt" && HOME="$fake_home" exec setsid "$wrapper" --worktree "$wt" -- sh worker-check.sh "$marker") >"$work/tree.out" 2>&1 &
launcher=$!
for _ in $(seq 1 50); do
  grep -q started "$work/tree.out" 2>/dev/null && break
  sleep 0.1
done
sandbox_pid="$(pgrep -f -o -- "bwrap .*$marker" || true)"
before="$(pgrep -f -- "$marker" | wc -l)"
if [[ -z "$sandbox_pid" || "$before" -lt 3 ]]; then
  fail "process-tree probe did not start (bwrap=$sandbox_pid processes=$before)"
else
  kill -KILL -- "-$sandbox_pid"
  gone=0
  for _ in $(seq 1 50); do
    if ! pgrep -f -- "$marker" >/dev/null; then gone=1; break; fi
    sleep 0.1
  done
  ((gone)) && pass "group SIGKILL of the runner leaves no process ($before before)" \
    || fail "processes survived the group kill: $(pgrep -af -- "$marker" || true)"
fi
wait "$launcher" 2>/dev/null || true
sandbox_pid=""

# Owner death: SIGKILL of the wrapper's parent also ends the sandbox.
bash -c 'cd "$1" && HOME="$2" "$3" --worktree "$1" -- sh worker-check.sh "$4"; :' owner \
  "$wt" "$fake_home" "$wrapper" "$marker" >"$work/parent.out" 2>&1 &
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
((gone)) && pass "SIGKILL of the runner's parent ends bwrap and the sandbox (--die-with-parent)" \
  || fail "sandbox survived its parent: $(pgrep -af -- "$marker" || true)"
wait "$owner" 2>/dev/null || true
rm -f -- "$worker_script"

if ((failures)); then
  printf 'verify-containment-test: %d check(s) FAILED\n' "$failures"
  exit 1
fi
printf 'verify-containment-test: all checks passed\n'
