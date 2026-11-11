#!/usr/bin/env python3
"""Run one nodecode turn through `nodecode rpc` and exit when it ends (issue #138).

Runs inside the sandbox that meshloop-nodecode-sandbox builds; it is plumbing, not a
boundary. Standard library only. Usage:

    nodecode-rpc-turn.py --nodecode PATH --model MODEL_REF --prompt-file FILE

The shell (`nodecode --model M --prompt-file F`) refuses to start without a terminal
(exit 2, "nodecode needs a terminal"), and `nodecode rpc` stops its organism as soon
as stdin closes, before a turn runs. So this driver sends one start_turn line, keeps
stdin open, reads gateway frames until the turn ends, then closes stdin.

MODEL_REF is `PROVIDER/MODEL[:EFFORT]` and the provider is required. The driver
splits it at the first `/` and names the provider in start_turn on its own; nodecode
splits a trailing `:EFFORT` off the model the way `--model` does. The provider is
explicit because nodecode splits a `PROVIDER/` prefix off a bare spec only for a
provider whose key comes from config, auth.json, OAuth or the environment, not from
the secret file the sandbox binds; unsplit, the whole spec would go to the default
lane (OpenCode Zen) as a model id. Prompt text travels inside one JSON string, never
through a shell.

Stdout: the assistant's messages, then one `nodecode-turn: <outcome>` line.
Exit codes: 0 turn.completed; 1 turn.failed; 2 usage; 3 start_turn rejected;
4 rpc ended or broke before the turn ended; 5 turn.cancelled.
"""

import argparse
import json
import subprocess
import sys
import time

# A fresh home has no models.dev catalog until its first fetch lands, and until then
# start_turn refuses a catalog provider ("... has not loaded yet"). Ask again, bounded.
CATALOG_WAIT_TRIES = 60
CATALOG_NOT_LOADED = "has not loaded yet"
END_EVENTS = {"turn.completed": 0, "turn.failed": 1, "turn.cancelled": 5}
TEXT_KEYS = ("text", "message", "content", "detail", "error", "reason")


def say(line):
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def payload_text(payload, limit=4000):
    """Readable text from an event payload: a known text field, else nothing."""
    if not isinstance(payload, dict):
        return ""
    for key in TEXT_KEYS:
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()[:limit]
        if isinstance(value, dict):
            nested = payload_text(value, limit)
            if nested:
                return nested
        if isinstance(value, list):
            parts = [v.get("text") for v in value if isinstance(v, dict)]
            joined = "\n".join(p for p in parts if isinstance(p, str))
            if joined.strip():
                return joined.strip()[:limit]
    return ""


def main():
    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("--nodecode", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--prompt-file", required=True)
    parser.add_argument("--raw-log", help="append every frame to this file (debugging)")
    try:
        args = parser.parse_args()
    except SystemExit:
        return 2
    provider, slash, model = args.model.strip().partition("/")
    if not slash or not provider or not model or provider.startswith("-"):
        say("nodecode-turn: usage: --model must be PROVIDER/MODEL[:EFFORT]")
        return 2
    try:
        with open(args.prompt_file, encoding="utf-8") as handle:
            prompt = handle.read()
    except OSError as error:
        say(f"nodecode-turn: usage: prompt file unreadable: {error.strerror}")
        return 2
    if not prompt.strip():
        say("nodecode-turn: usage: prompt file is empty")
        return 2

    def start_turn(attempt):
        command = "meshloop-turn-{}".format(attempt)
        line = json.dumps(
            {
                "type": "start_turn",
                "command_id": command,
                "request": {"prompt": prompt, "provider": provider, "model": model},
            }
        )
        rpc.stdin.write(line + "\n")
        rpc.stdin.flush()
        return command

    try:
        raw = open(args.raw_log, "a", encoding="utf-8") if args.raw_log else None
    except OSError as error:
        say(f"nodecode-turn: usage: raw log unwritable: {error.strerror}")
        return 2
    rpc = subprocess.Popen(
        [args.nodecode, "rpc", "--ephemeral"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=None,
        text=True,
        encoding="utf-8",
        bufsize=1,
    )
    outcome, code = "rpc ended before the turn ended", 4
    try:
        attempt = 1
        command_id = start_turn(attempt)
        for text in rpc.stdout:
            if raw:
                raw.write(text)
                raw.flush()
            try:
                frame = json.loads(text)
            except ValueError:
                continue
            payload = frame.get("payload") or {}
            kind = payload.get("type")
            if kind == "gateway_turn_command_result":
                result = payload.get("result") or {}
                if result.get("command_id") == command_id and result.get("status") == "rejected":
                    if (
                        CATALOG_NOT_LOADED in result.get("message", "")
                        and attempt < CATALOG_WAIT_TRIES
                    ):
                        time.sleep(1)
                        attempt += 1
                        command_id = start_turn(attempt)
                        continue
                    outcome = "rejected {}: {}".format(
                        result.get("code", "?"), result.get("message", "")
                    )
                    code = 3
                    break
            elif kind == "sync_protocol_error":
                outcome, code = "protocol error: " + payload_text(payload), 4
                break
            elif kind == "session_stream_event":
                event = (payload.get("frame") or {}).get("event") or {}
                etype = event.get("type", "")
                body = event.get("payload")
                if etype == "turn.provider_retry" and isinstance(body, dict):
                    say(
                        "nodecode-turn: provider retry {} (status {})".format(
                            body.get("attempt", "?"), body.get("status", "?")
                        )
                    )
                elif etype == "turn.usage" and isinstance(body, dict):
                    say(
                        "nodecode-turn: usage {}/{} input {} output {} cost_usd {}".format(
                            body.get("provider", "?"),
                            body.get("model", "?"),
                            body.get("input-tokens", "?"),
                            body.get("output-tokens", "?"),
                            body.get("cost_usd", "?") if body.get("cost_known") else "unknown",
                        )
                    )
                elif etype == "turn.assistant_message_completed":
                    message = payload_text(body, limit=1 << 20)
                    if message:
                        say(message)
                elif etype in END_EVENTS:
                    code = END_EVENTS[etype]
                    detail = payload_text(body) if code else ""
                    outcome = etype + (": " + detail if detail else "")
                    break
    except BrokenPipeError:
        outcome, code = "rpc closed its input before the turn ended", 4
    finally:
        try:
            rpc.stdin.close()
        except OSError:
            pass
        try:
            rpc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            rpc.kill()
            rpc.wait()
        if raw:
            raw.close()
    say(f"nodecode-turn: {outcome}")
    return code


if __name__ == "__main__":
    sys.exit(main())
