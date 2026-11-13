"""Measure fresh-session and warm hop hints, including complete PTY output.

After the `f` actions, `g f` filtered hints: each filter letter is checked in
its full PTY output for Kitty PNG transmissions and atlas placements."""

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
    reads = [at for at, event in events if event.startswith("input read Key")]
    writes = [at for at, event in events if event == "hints written"]
    samples = []
    cursor = 0.0
    for label, key, size, extra in actions:
        # Each action from its own key read to its own hints written: no
        # other key read in between.
        start = next(
            at for at, event in events
            if at > cursor and event.startswith("input read Key")
            and re.search(rf"code: Char\('{key}'\), modifiers: KeyModifiers\(0x0\)", event)
        )
        end = next(at for at in writes if at > start)
        assert not any(start < at < end for at in reads), f"{label}: another key before hints written"
        samples.append({"action": label, "ms": end - start, "bytes": size, **extra})
        cursor = start
    return samples


def graphics(raw, state):
    """Kitty graphics commands in `raw`: PNG transmissions (f=100 by a=t, a=T
    or no a=, and their continuation payloads) and hint placements (a=p)."""
    png, placed = 0, []
    for match in re.finditer(rb"\x1b_G([^;\x1b]*)(?:;[^\x1b]*)?\x1b\\", raw):
        keys = dict(kv.split(b"=", 1) for kv in match[1].split(b",") if b"=" in kv)
        if state["more"]:
            png += state["more_png"]
            state["more"] = keys.get(b"m") == b"1"
            continue
        action = keys.get(b"a", b"t")
        if action in (b"t", b"T"):
            is_png = keys.get(b"f") == b"100"
            png += is_png
            if is_png:
                state["uploaded"].add(keys.get(b"i"))
            state["more"], state["more_png"] = keys.get(b"m") == b"1", is_png
        elif action == b"p":
            placed.append(keys.get(b"i"))
    return png, placed


def run(binary, scale):
    page = (
        "<title>hint benchmark</title>"
        "<body style='font:18px sans-serif'>"
        + "<a href=%23>link</a> words " * 200
        + "<p>scroll space</p>" * 80
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
        state = {"uploaded": set(), "more": False, "more_png": False}

        def press(label, keys, filtering=False):
            os.write(fd, keys)
            raw = pump(0.3)
            uploaded = set(state["uploaded"])
            png, placed = graphics(raw, state)
            # Hint placements: of a PNG image (page tiles are raw pixels).
            placed = [i for i in placed if i in state["uploaded"]]
            if label:
                assert b"a=T" in raw or b"a=p" in raw, f"{label}: no hint pixels"
                extra = {}
                if filtering:
                    shown = re.findall(rb"filter: [^\x1b]*?(\d+) matches", raw)
                    extra = {
                        "png": png,
                        "placed": len(placed),
                        "matches": int(shown[-1]) if shown else None,
                        "atlas_before": bool(placed) and all(i in uploaded for i in placed),
                    }
                actions.append((label, keys.decode()[-1], len(raw), extra))

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
            press(None, b"\x1b")
            press(None, b"g")
            press("g f", b"f")
            for letter in b"link":
                press("filter letter", bytes([letter]), filtering=True)
            press(None, b"\x1b")
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
    letters = [sample for sample in samples if sample["action"] == "filter letter"]
    print(
        f"filter letter    PNG transmissions {sum(s['png'] for s in letters)}"
        f"  placed == matches {sum(s['placed'] == s['matches'] for s in letters)}/{len(letters)}"
        f"  matches min {min((s['matches'] or 0) for s in letters)}"
        f"  a=p of atlas uploaded before {sum(s['atlas_before'] for s in letters)}/{len(letters)}"
    )
    assert all(
        s["png"] == 0 and (s["matches"] or 0) > 1 and s["placed"] == s["matches"] and s["atlas_before"]
        for s in letters
    ), "a filter letter sent a PNG, placed a filtered-out hint, or left one match"
    if args.check:
        first = [sample["ms"] for sample in samples if sample["action"] == "first f"]
        assert percentile(first, .9) < 10, "first-f p90 is not below 10 ms"
