"""Measure fresh-session and warm hop hints, including complete PTY output.

After the `f` actions, `g f` filtered hints: each filter key is read until its
`N matches` status line and quiet output, and checked for Kitty PNG
transmissions and for the exact atlas parts placed. Then a second page with
mixed link texts: a text-filter letter that removes some hints."""

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


def codes(n, keys="1234567890"):
    """hints.js scan(): prefix-free codes, the first targets in reading order shortest."""
    c, off = [""], 0
    while len(c) - off < n or len(c) == 1:
        c += [c[off] + k for k in keys]
        off += 1
    return c[off:off + n]


def identity_error(placed, want):
    """None if `placed` is exactly the atlas parts `want`, each once."""
    if len(placed) != len(set(placed)):
        return "a hint placed twice"
    if set(placed) != set(want):
        return f"placed {len(set(placed) - set(want))} filtered-out, missed {len(set(want) - set(placed))}"
    return None


def identity_self_check():
    parts = [(b"3", b"0", str(y).encode(), b"9", b"9") for y in range(0, 60, 10)]
    want = parts[::2]
    assert identity_error(want, want) is None
    assert identity_error(parts[:3], want), "a wrong subset of the right count passed"
    assert identity_error(want[:2] + want[:1], want), "a duplicate of the right count passed"


def drain_self_check():
    """An early `0 matches` status line, then the overlay late and in pieces:
    drain waits for every placement."""
    status = b"\x1b[40;1Hfilter:   %d matches"
    script = [status % 0, b"", b"", b"", b"\x1b_Ga=d,d=i,i=3,q=2\x1b\\", b"",
              b"\x1b_Ga=p,i=3,p=1,x=0,y=0,w=9,h=9\x1b\\" + status % 2, b"", b"",
              b"\x1b_Ga=p,i=3,p=2,x=0,y=10,w=9,h=9\x1b\\", b"", b""]
    now = [0.0]

    def read(seconds):
        now[0] += seconds
        return script.pop(0) if script else b""

    output = drain(read, clock=lambda: now[0])
    assert output.count(b"a=p,i=3,") == 2, f"drain ended before every placement: {output!r}"


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
            # The atlas source rect: which hint's part this is.
            placed.append(tuple(keys.get(k) for k in (b"i", b"x", b"y", b"w", b"h")))
    return png, placed


# Links on the mixed page, all on screen.
MIXED = 16


def run(binary, scale):
    page = (
        "<title>hint benchmark</title>"
        "<body style='font:18px sans-serif'>"
        + "<a href=%23>link</a> words " * 200
        + "<p>scroll space</p>" * 80
    )
    page = "data:text/html;base64," + base64.b64encode(page.encode()).decode()
    mixed = "<title>mixed hints</title><body style='font:18px sans-serif'>" + "".join(
        f"<a href=%23{k}>{'alpha' if k % 2 == 0 else 'beta'} {k}</a><br>" for k in range(MIXED)
    )
    mixed = "data:text/html;base64," + base64.b64encode(mixed.encode()).decode()
    with tempfile.TemporaryDirectory(prefix="webskiff-hints-") as home:
        pid, fd = pty.fork()
        if pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 1920, 1280))
            os.environ["HOME"] = home
            os.environ["ZWEB_TRACE"] = "1"
            os.execv(binary, [binary, "--gfx", "kitty", "--scale", str(scale), page])

        def drain(seconds=3, quiet=0.1):
            """Output until a `N matches` status line, then `quiet` s of none."""
            output = bytearray()
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                chunk = pump(min(quiet, max(deadline - time.monotonic(), 0)))
                output.extend(chunk)
                if not chunk and re.search(rb"filter: [^\x1b]*?\d+ matches", output):
                    return bytes(output)
            raise AssertionError(f"no filter status line in {seconds} s: {bytes(output[-1000:])!r}")

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

        def press(label, keys, filtering=False, want=None):
            os.write(fd, keys)
            raw = drain() if filtering else pump(0.3)
            uploaded = set(state["uploaded"])
            png, placed = graphics(raw, state)
            # Hint placements: of a PNG image (page tiles are raw pixels).
            placed = [p for p in placed if p[0] in state["uploaded"]]
            if label:
                assert b"a=T" in raw or b"a=p" in raw, f"{label}: no hint pixels"
                extra = {}
                if filtering:
                    shown = re.findall(rb"filter: [^\x1b]*?(\d+) matches", raw)
                    extra = {
                        "png": png,
                        "placed": len(placed),
                        "matches": int(shown[-1]) if shown else None,
                        "atlas_before": bool(placed) and all(p[0] in uploaded for p in placed),
                        "identity": identity_error(placed, want(placed)) if want else "no expected set",
                    }
                actions.append((label, keys.decode()[-1], len(raw), extra))
            return placed, raw

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
            gf, raw = press("g f", b"f", filtering=True)
            # No cell skipped off-screen: index k = hint k (render.rs hop()).
            shown = re.findall(rb"filter: [^\x1b]*?(\d+) matches", raw)
            assert len(gf) == int(shown[-1]), f"g f placed {len(gf)} of {shown[-1]!r} hints"
            for letter in b"link":
                # Every link says "link": each letter keeps every hint.
                press("filter letter", bytes([letter]), filtering=True, want=lambda _: gf)
            # A digit (code prefix) filters hints out.
            digit = {r for r, k in zip(gf, codes(len(gf))) if k.startswith("1")}
            press("filter digit", b"1", filtering=True, want=lambda _: digit)
            press(None, b"\x1b")
            # Mixed link texts, after the legacy steps: a letter keeps some.
            os.write(fd, b"o")
            pump(0.3)
            os.write(fd, b"\x1b[200~" + mixed.encode() + b"\x1b[201~\r")
            output, deadline = b"", time.monotonic() + 20
            while b"mixed hints" not in output and time.monotonic() < deadline:
                output += pump(0.1)
            assert b"mixed hints" in output, f"mixed page did not load: {output[-1000:]!r}"
            pump(1)
            press(None, b"g")
            mgf, raw = press(None, b"f", filtering=True)
            shown = re.findall(rb"filter: [^\x1b]*?(\d+) matches", raw)
            assert len(mgf) == int(shown[-1]) == MIXED, f"mixed g f placed {len(mgf)} of {shown[-1]!r}"
            alpha = set(mgf[::2])  # "alpha k" at even k: the hints with a "p"
            press("text filter letter", b"p", filtering=True, want=lambda _: alpha)
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
    identity_self_check()
    drain_self_check()
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
    digits = [sample for sample in samples if sample["action"] == "filter digit"]
    texts = [sample for sample in samples if sample["action"] == "text filter letter"]
    print(
        f"filter digit     PNG transmissions {sum(s['png'] for s in digits)}"
        f"  placed == matches {sum(s['placed'] == s['matches'] for s in digits)}/{len(digits)}"
        f"  matches {min(s['matches'] or 0 for s in digits)}-{max(s['matches'] or 0 for s in digits)}"
        f" of {min(s['matches'] or 0 for s in letters)}+ after the letters"
    )
    print(
        f"filter letter    PNG transmissions {sum(s['png'] for s in letters)}"
        f"  placed == matches {sum(s['placed'] == s['matches'] for s in letters)}/{len(letters)}"
        f"  matches min {min((s['matches'] or 0) for s in letters)}"
        f"  a=p of atlas uploaded before {sum(s['atlas_before'] for s in letters)}/{len(letters)}"
    )
    print(
        f"text filter      PNG transmissions {sum(s['png'] for s in texts)}"
        f"  matches {sorted({s['matches'] for s in texts})} of {MIXED}"
        f"  a=p of atlas uploaded before {sum(s['atlas_before'] for s in texts)}/{len(texts)}"
    )
    wrong = [s["identity"] for s in letters + digits + texts if s["identity"]]
    print(f"atlas parts      exact set placed {len(letters + digits + texts) - len(wrong)}/{len(letters + digits + texts)}"
          f"  {wrong[:3]}")
    assert all(
        s["png"] == 0 and (s["matches"] or 0) > 1 and s["placed"] == s["matches"] and s["atlas_before"]
        and s["identity"] is None
        for s in letters + digits + texts
    ), "a filter key sent a PNG, placed a wrong or duplicate hint, or left one match"
    assert all(1 < s["matches"] < MIXED for s in texts), "the text filter did not remove some hints"
    if args.check:
        first = [sample["ms"] for sample in samples if sample["action"] == "first f"]
        assert percentile(first, .9) < 10, "first-f p90 is not below 10 ms"
