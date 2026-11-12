"""Measure fresh-session and warm hop hints, including complete PTY output."""

import argparse
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import tempfile
import termios
import time


def percentile(values, fraction):
    return sorted(values)[min(int(len(values) * fraction), len(values) - 1)]


def trace_samples(trace, actions):
    events = [
        (float(match[1]), match[2])
        for match in re.finditer(r"^T +([\d.]+) (.*)$", trace, re.M)
    ]
    starts = iter(
        at for at, event in events
        if event.startswith("input read Key") and re.search(r"Char\('[fa]'\)", event)
    )
    writes = [at for at, event in events if event == "hints written"]
    samples = []
    for label, size in actions:
        start = next(starts)
        end = next(at for at in writes if at > start)
        samples.append({"action": label, "ms": end - start, "bytes": size})
    return samples


def run(binary, scale):
    page = (
        "<title>hint benchmark</title>"
        "<body style='font:18px sans-serif'>"
        + "<a href=%23>link</a> words " * 200
    )
    page = "data:text/html;base64," + base64.b64encode(page.encode()).decode()
    with tempfile.TemporaryDirectory(prefix="webskiff-hints-") as home:
        pid, fd = pty.fork()
        if pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 1920, 1280))
            os.environ["HOME"] = home
            os.environ["ZWEB_TRACE"] = "1"
            os.execv(binary, [binary, "--gfx", "kitty", "--scale", str(scale), page])

        def pump(seconds):
            output = bytearray()
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if select.select([fd], [], [], 0.002)[0]:
                    try:
                        chunk = os.read(fd, 1 << 20)
                    except OSError:
                        break
                    if not chunk:
                        break
                    output.extend(chunk)
            return bytes(output)

        actions = []

        def press(label, keys):
            os.write(fd, keys)
            raw = pump(0.3)
            if label:
                assert b"a=T" in raw or b"a=p" in raw, f"{label}: no hint pixels"
                actions.append((label, len(raw)))

        try:
            output = pump(0.5)
            os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c")
            deadline = time.monotonic() + 20
            while b"hint benchmark" not in output and time.monotonic() < deadline:
                output += pump(0.1)
            assert b"hint benchmark" in output, f"benchmark page did not load: {output[-1000:]!r}"
            pump(1)
            press("first f", b"f")
            press("hint letter", b"a")
            press(None, b"\x1b")
            for _ in range(4):
                press("later f", b"f")
                press(None, b"\x1b")
            press(None, b"j")
            press("f after scroll", b"f")
            press(None, b"\x11")
            _, status = os.waitpid(pid, 0)
            assert status == 0, f"browser exit status: {status}"
        finally:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            else:
                os.waitpid(pid, 0)
            os.close(fd)

        return trace_samples((Path(home) / ".cache/zweb/zweb.log").read_text(), actions)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=lambda path: str(Path(path).resolve()))
    parser.add_argument("--scale", type=int, choices=(1, 2), default=1)
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--check", action="store_true", help="require first-f p90 below 10 ms")
    args = parser.parse_args()
    samples = [sample for _ in range(args.runs) for sample in run(args.binary, args.scale)]
    print(json.dumps({"scale": args.scale, "samples": samples}))
    for action in dict.fromkeys(sample["action"] for sample in samples):
        group = [sample for sample in samples if sample["action"] == action]
        times = [sample["ms"] for sample in group]
        sizes = [sample["bytes"] for sample in group]
        print(
            f"{action:16} p50/p90 {percentile(times, .5):.2f}/{percentile(times, .9):.2f} ms"
            f"  bytes {percentile(sizes, .5)}/{percentile(sizes, .9)} (n={len(group)})"
        )
    if args.check:
        first = [sample["ms"] for sample in samples if sample["action"] == "first f"]
        assert percentile(first, .9) < 10, "first-f p90 is not below 10 ms"
