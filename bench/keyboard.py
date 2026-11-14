"""Exercise the browser's real keyboard dispatch and measure PTY output."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import sys
import tempfile
import termios
import time


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary")
parser.add_argument("--check", action="store_true")
parser.add_argument("--config-check", action="store_true")
parser.add_argument("--check-only", action="store_true")
parser.add_argument("--enhanced-check", action="store_true")
parser.add_argument("--hint-check", action="store_true")
args = parser.parse_args()
binary = str(Path(args.binary).resolve())
fixture = Path(__file__).with_suffix(".html").resolve().as_uri()
home = tempfile.TemporaryDirectory(prefix="webskiff-keyboard-")
if args.config_check:
    config = Path(home.name) / ".config/webskiff/config.toml"
    config.parent.mkdir(parents=True)
    config.write_text(
        "[keys.browse]\nfind = ['z']\n"
        "[keys.global]\nfind = ['Ctrl-S']\n"
        "[keys.prompt]\nclear = ['Ctrl-Y']\n"
    )
# External editor (g e): one script; the file "mode" picks what it does.
editor_dir = Path(home.name) / "editor"
edit_tmp = Path(home.name) / "tmp"
editor_dir.mkdir()
edit_tmp.mkdir()
editor = editor_dir / "editor.sh"
editor.write_text(
    "#!/bin/sh\n"
    f"d='{editor_dir}'\n"
    'touch "$d/ran"\n'
    'printf %s "$1" > "$d/path"\n'
    'case "$(cat "$d/mode")" in\n'
    "  ok) printf 'hello-ext\\n' > \"$1\";;\n"
    "  fail) printf 'changed\\n' > \"$1\"; exit 3;;\n"
    "  wait) echo start >> \"$d/log\"; read -r k; printf %s \"$k\" > \"$d/keys\"; echo end >> \"$d/log\";;\n"
    "esac\n"
)
editor.chmod(0o700)
if args.hint_check:
    # Page one: 12 links. Keys typed in insert mode make it busy (b), load
    # itself later (m), or load page two while busy (n): hint replies come
    # late or a load commits while `g f` is open.
    links = "".join(f"<a href=#{i} onclick=\"document.title='picked {i}';return false\">item {i}</a><br>" for i in range(12))
    Path(home.name, "a.html").write_text(
        f"<title>keyboard fixture</title>{links}<script>const spin=()=>{{const t=Date.now()+1500;while(Date.now()<t);}};"
        "addEventListener('keydown',e=>{if(e.key=='b')setTimeout(spin,0);"
        "if(e.key=='m')setTimeout(()=>location.href='a.html?2',1000);"
        "if(e.key=='n')setTimeout(()=>{location.href='b.html';spin()},0)})</script>"
    )
    Path(home.name, "b.html").write_text(
        "<title>page two</title><a href=#s onclick=\"document.title='picked solo';return false\">solo</a>"
        "<script>addEventListener('keydown',e=>{if(e.key=='w')setTimeout(()=>location.href='c.html',1000)})</script>"
    )
    # Page three loads for 1.5 s after it commits.
    Path(home.name, "c.html").write_text(
        "<title>slow page</title><a href=#>slow</a><script>const t=Date.now()+1500;while(Date.now()<t);</script>"
    )
    fixture = Path(home.name, "a.html").as_uri()
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 1920, 1280))
    os.environ["HOME"] = home.name
    if args.check_only:
        os.environ.pop("VISUAL", None)
        os.environ["EDITOR"] = str(editor)
        os.environ["TMPDIR"] = str(edit_tmp)
    if args.check or args.check_only or args.config_check or args.enhanced_check or args.hint_check:
        # Stage times in the log printed on a failure; not for timing runs.
        os.environ["ZWEB_TRACE"] = "1"
    os.execv(binary, [binary, "--gfx", "kitty", fixture])
original_flags = termios.tcgetattr(fd)[3]


def pump(seconds):
    output = []
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if select.select([fd], [], [], 0.002)[0]:
            try:
                data = os.read(fd, 1 << 20)
            except OSError:
                break
            output.append((time.monotonic(), data))
    return output


samples = {}


def press(keys, label=None, expected=None):
    pump(0.05)
    start = time.monotonic()
    os.write(fd, keys)
    output = pump(0.25)
    raw = b"".join(data for _, data in output)
    images = [at for at, data in output if b"a=T" in data or b"a=p" in data]
    if label:
        samples.setdefault(label, []).append({
            "first_output_ms": (output[0][0] - start) * 1000 if output else None,
            "first_pixels_ms": (images[0] - start) * 1000 if images else None,
            "bytes": len(raw),
        })
    if expected is not None:
        assert expected.encode() in raw, f"{keys!r}: expected {expected!r}, got {raw[-3000:]!r}"
    return raw


def wait_for(expected, seconds=10):
    output = bytearray()
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline and expected.encode() not in output:
        output.extend(b"".join(data for _, data in pump(0.05)))
    assert expected.encode() in output, f"expected {expected!r}, got {bytes(output[-3000:])!r}"
    return bytes(output)


def external_edit(mode, expected):
    (editor_dir / "mode").write_text(mode)
    press(b"g")
    os.write(fd, b"e")
    return wait_for(expected)


def percentile(values, fraction):
    values = sorted(value for value in values if value is not None)
    return values[min(int(len(values) * fraction), len(values) - 1)] if values else None


# A complete Kitty graphics command: its control keys, an optional payload.
KITTY = re.compile(rb"\x1b_G([^;\x1b]*)(?:;[^\x1b]*)?\x1b\\")
# A tab picker row: "<number>: <title or URL>" written at column 3.
PICKER_ROW = re.compile(rb"\x1b\[\d+;3H(\d+): ([^\x1b\r\n]*)")
# The active tab's label in the tab bar (reverse video).
ACTIVE_TAB = re.compile(rb"\x1b\[7m (\d+)\*? ")


def page_image(raw):
    return any({b"a=T", b"a=p"} & set(m.group(1).split(b",")) for m in KITTY.finditer(raw))


def picker_rows(raw):
    return [(int(number), name.decode()) for number, name in PICKER_ROW.findall(raw)]


def active_tab(raw):
    tabs = ACTIVE_TAB.findall(raw)
    assert tabs, f"no active tab in the browser chrome: {raw[-2000:]!r}"
    return int(tabs[-1])


def drain():
    deadline = time.monotonic() + 2
    while pump(0.03) and time.monotonic() < deadline:
        pass


def until(keys, found, label=None, seconds=1.0):
    """Writes keys (or calls `keys`) after a drain; returns all output up to
    `found` and 50 ms more.

    The sample is from the key write to the read that completes `found`."""
    drain()
    buffer = bytearray()
    start = time.monotonic()
    keys() if callable(keys) else os.write(fd, keys)
    while time.monotonic() < start + seconds:
        if select.select([fd], [], [], 0.002)[0]:
            buffer.extend(os.read(fd, 1 << 20))
            if found(bytes(buffer)):
                if label:
                    samples.setdefault(label, []).append({"ms": (time.monotonic() - start) * 1000})
                buffer.extend(b"".join(data for _, data in pump(0.05)))
                return bytes(buffer)
    raise AssertionError(f"{keys!r}: expected output not seen in {seconds} s: {bytes(buffer[-3000:])!r}")


def wait_for(marker, seconds=10):
    output = bytearray()
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        output.extend(b"".join(data for _, data in pump(0.1)))
        if marker in output:
            return bytes(output)
    raise AssertionError(f"{marker!r} not seen: {bytes(output[-3000:])!r}")


def open_new_tab(url, marker):
    press(b"t", expected="NEW TAB")
    os.write(fd, url.encode() + b"\r")
    try:
        return wait_for(marker, 5)
    except AssertionError as first:
        # Engine bug: a new tab sometimes stays stuck loading. Reload the same
        # tab once, visibly; a second miss fails with both errors.
        print(f"RETRY: first load of {url} not seen in 5 s; reloading the same tab. "
              f"First attempt: {first}", file=sys.stderr, flush=True)
        press(b"o", expected=" OPEN ")
        os.write(fd, url.encode() + b"\r")
        try:
            return wait_for(marker)
        except AssertionError as reload:
            raise AssertionError(f"first attempt: {first}; reload: {reload}") from reload


def open_titled_tab(title):
    return open_new_tab(fixture + "?title=" + title, f" {title} ".encode())


def pick_tab(before, query, target, cancel=False, timed=False):
    """Opens the tab picker in tab `before`, types `query`, which must list only
    tab `target`, and closes with Esc or Enter. Returns the active tab after."""
    raw = until(b"T", lambda raw: bool(picker_rows(raw)), timed and "picker open")
    assert b" TABS " in raw, f"picker did not open: {raw[-2000:]!r}"
    assert active_tab(raw) == before, f"active tab {active_tab(raw)} before the picker, not {before}"
    for index, character in enumerate(query):
        raw = until(character.encode(), lambda raw: bool(picker_rows(raw)), timed and index == 0 and "picker filter")
    if query:
        rows = picker_rows(raw)
        assert [number for number, _ in rows] == [target], f"{query!r} lists {rows}, not only tab {target}"
    key, label = (b"\x1b", "picker esc close") if cancel else (b"\r", "picker enter close")
    raw = until(key, page_image, timed and label)
    assert b" BROWSE " in raw and b" TABS " not in raw.split(b" BROWSE ")[-1], f"picker did not close: {raw[-2000:]!r}"
    after = active_tab(raw)
    assert after == (before if cancel else target), f"active tab {after}, before {before}, target {target}"
    return after


def set_size(rows, cols):
    """Resizes the terminal, 16x32 pixel cells as at start; the browser gets SIGWINCH."""
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * 16, rows * 32))


def picker_survives_resize(active, tabs):
    """A resize with the picker open redraws its rows for the new size, with
    the page images still hidden; Esc then shows the page again."""
    until(b"T", lambda raw: bool(picker_rows(raw)))
    for rows, cols in ((30, 100), (50, 120), (40, 120)):
        # The last page row (rows - 1) is cleared before the rows are drawn.
        last = f"\x1b[{rows - 1};1H\x1b[2K".encode()
        raw = until(lambda: set_size(rows, cols),
                    lambda raw: last in raw and len(picker_rows(raw)) == tabs, seconds=3)
        assert not page_image(raw), f"page image shown with the picker open at {rows}x{cols}"
    raw = until(b"\x1b", page_image)
    assert active_tab(raw) == active, f"active tab {active_tab(raw)} after Esc, not {active}"


def pick_retitled_tab():
    """The picker lists a tab once its new title matches; Enter picks it."""
    open_new_tab(fixture + "?retitle", b" Quokka ")
    until(b"T", lambda raw: bool(picker_rows(raw)))
    os.write(fd, b"zbr")
    rows = picker_rows(b"".join(data for _, data in pump(0.3)))
    assert rows == [], f"'zbr' lists {rows} before the title change"
    wait_for(b"\x1b[2;3H2: Zebra")
    raw = until(b"\r", page_image)
    assert active_tab(raw) == 2, f"active tab {active_tab(raw)} after Enter, not 2"
    press(b"x", expected="\x1b[7m 1")


def wait_for_fixture(text=b"keyboard fixture", keys=None):
    output = bytearray()
    deadline = time.monotonic() + 20
    if keys is not None:
        os.write(fd, keys)
    while time.monotonic() < deadline:
        if not select.select([fd], [], [], max(0, deadline - time.monotonic()))[0]:
            break
        try:
            data = os.read(fd, 1 << 20)
        except OSError:
            break
        if not data:
            break
        output.extend(data)
        # Startup keys need an image after the title; jump checks only need their marker.
        title = output.find(text)
        if title >= 0 and (keys is not None or b"a=T" in output[title:]):
            return
    raise AssertionError(f"{text!r} did not appear: {bytes(output[-3000:])!r}")


try:
    pump(0.5)
    os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c")
    if args.enhanced_check:
        negotiation = b"".join(data for _, data in pump(0.25))
        assert b"\x1b[?u" in negotiation, f"no enhanced-keyboard query: {negotiation!r}"
        os.write(fd, b"\x1b[?1u\x1b[?62;c")
    if args.check or args.check_only or args.config_check or args.enhanced_check or args.hint_check:
        wait_for_fixture()
    else:
        pump(4)
    if args.enhanced_check:
        raw_flags = termios.ICANON | termios.ECHO | termios.ISIG
        assert termios.tcgetattr(fd)[3] & raw_flags == 0, "browser did not enter raw mode"
        press(b"/", expected="FIND")
        press(b"B", expected="m first:11:b")
        for index, character in enumerate("rowser", 2):
            press(character.encode(), expected=f"m browser:0:{'Browser'[:index]}")
        raw = press(b"\x1b[99;5u", expected="m browser:0:Browser")
        assert b"^[[99;5u" not in raw, "enhanced Ctrl-C was echoed"
        for sequence in (b"\x1b[<35;1;59M", b"\x1b[<35;30;10M"):
            raw = press(sequence)
            assert sequence not in raw and b"^[[" not in raw, "mouse reports were echoed"
        press(b"\r")
        press(b"n", expected="m browser:8:Browser")
        press(b"N", expected="m browser:0:Browser")
        press(b"/")
        press(b"\x1b[27u", expected="BROWSE")
        assert termios.tcgetattr(fd)[3] & raw_flags == 0, "raw mode was lost during search"
        # Jumplist: Ctrl-O goes back and Ctrl-I forward, within a tab and across tabs.
        press(b"o", expected="OPEN")
        wait_for_fixture(b"\x1b[7m 1 jump page", (fixture + "?jump").encode() + b"\r")
        wait_for_fixture(b"keyboard fixture", b"\x1b[111;5u")
        wait_for_fixture(b"jump page", b"\x1b[105;5u")
        press(b"t", expected="NEW TAB")
        wait_for_fixture(b"\x1b[7m 2 keyboard fixture", (fixture + "?tab=2").encode() + b"\r")
        wait_for_fixture(b"\x1b[7m 1", b"\x1b[111;5u")
        wait_for_fixture(b"\x1b[7m 2", b"\x1b[105;5u")
        press(b"\x1b[113;5u")
        pump(2)
        assert termios.tcgetattr(fd)[3] & raw_flags == original_flags & raw_flags, "terminal flags were not restored"
        print("ENHANCED KEYBOARD AND TERMINAL RESTORATION CHECKS PASSED")
        raise SystemExit(0)
    if args.hint_check:
        def keys(*sequence):
            for k in sequence:
                os.write(fd, k)
                time.sleep(0.05)

        def drain(until=None, seconds=8):
            """Output until `until` shows, then until 0.4 s of none."""
            output, deadline = bytearray(), time.monotonic() + seconds
            while until is not None and until not in output and time.monotonic() < deadline:
                output.extend(b"".join(data for _, data in pump(0.05)))
            while time.monotonic() < deadline:
                chunk = b"".join(data for _, data in pump(0.4))
                if not chunk:
                    break
                output.extend(chunk)
            return bytes(output)

        def shown(raw):
            """The last `filter: N matches`, and hints placed after the last overlay change."""
            matches = re.findall(rb"filter: [^\x1b]*?(\d+) matches", raw)
            last = raw.rfind(b"\x1b_Ga=d,d=i,i=3,")
            return int(matches[-1]) if matches else None, len(re.findall(rb"a=p,i=3,", raw[last:])) if last >= 0 else None

        def arm(key):
            keys(b"i", key, b"\x1b")
            pump(0.2)

        # F2: a late reply of an earlier session is not this session's.
        arm(b"b")
        keys(b"g", b"f", b"\x1b", b"g", b"f", b"1")
        got = shown(drain(b"3 matches"))
        assert got == (3, 3), f"g f, Esc, g f: a stale reply replaced the filtered hints: {got}"
        keys(b"\x1b")
        arm(b"b")
        keys(b"f", b"\x1b", b"g", b"f", b"1")
        got = shown(drain(b"3 matches"))
        assert got == (3, 3), f"f, Esc, g f: a stale reply dropped the queued digit: {got}"
        keys(b"\x1b")
        # F1: keys of a session that a load ended do not apply to the next one.
        arm(b"m")
        keys(b"g", b"f")
        drain(b"12 matches")
        keys(b"1")
        drain(b"a.html?2")
        keys(b"g", b"f")
        assert shown(drain(b"matches"))[0] == 12, "a digit typed before a load still filters the next g f"
        keys(b"\x1b")
        arm(b"n")
        keys(b"g", b"f", b"1", b"\r")
        drain(b"page two")
        keys(b"g", b"f")
        raw = drain(b"matches")
        assert b"picked" not in raw, "keys queued before a load picked a hint in the next g f"
        # F5: with one hint left, only an accepted key or Enter follows it.
        for key in (b"\x7f", b"z"):
            keys(key)
            assert b"picked" not in drain(), f"{key!r} followed the only hint"
        keys(b"\r")
        assert b"picked solo" in drain(b"picked solo"), "Enter did not follow the only hint"
        keys(b"g", b"f")
        drain(b"matches")
        keys(b"s")
        assert b"picked solo" in drain(b"picked solo"), "an accepted letter did not follow the only hint"
        # F6: a load that commits while `g f` is open reads hints ahead only
        # once it has finished: one round, letters and digits.
        arm(b"w")
        log = Path(home.name) / ".cache/zweb/zweb.log"
        start = log.stat().st_size
        keys(b"g", b"f")
        drain(b"matches")
        drain(b"slow page")
        pump(2.5)
        with log.open("rb") as f:
            f.seek(start)
            reads = f.read().count(b"hints prepare\n")
        assert reads == 2, f"hints read ahead {reads} times (not 2) after a load that committed during g f"
        print("HINT CHECKS PASSED")
        raise SystemExit(0)
    if args.config_check:
        assert b" FIND " not in press(b"/"), "disabled default still opens find"
        press(b"z", expected="FIND")
        press(b"alpha", expected="m first:0:alpha")
        press(b"\r")
        press(b"n", expected="m first:6:alpha")
        press(b"\x13", expected="FIND")
        press(b"\x1b")
        press(b"\x0c", expected="OPEN")
        press(b"word")
        press(b"\x19", expected="Address or web search")
        press(b"\x1b")
        press(b"\x1bOP", expected="Ctrl-S")
        press(b"G", expected="HELP MODE")
        press(b"\x1b")
        print("CONFIGURED KEYBOARD CHECKS PASSED")
        raise SystemExit(0)
    if args.check or args.check_only:
        # Esc without a prefix reaches the page; Esc after a prefix only cancels it.
        press(b"\x1b", expected="escape 1")
        raw = press(b"g")
        for description in ("Go to the top", "Focus the first text field"):
            assert description.encode() in raw, f"prefix popup lacks {description!r}: {raw[-3000:]!r}"
        raw = press(b"\x1b") + press(b"i", expected="INSERT")
        assert b"escape 2" not in raw, "Esc after a prefix was sent to the page"
        press(b"\x1b", expected="BROWSE")
        press(b"/")
        press(b"alpha", expected="m first:0:alpha")
        press(b"\r", expected="m first:0:alpha")
        press(b"n", expected="m first:6:alpha")
        press(b"n", expected="m first:15:alpha")
        press(b"n", expected="m second:0:alpha")
        press(b"n", expected="m first:0:alpha")
        press(b"N", expected="m second:0:alpha")
        press(b"n", expected="m first:0:alpha")
        press(b"/")
        press(b"alphabet", expected="m first:6:alphabet")
        press(b"\x7f", expected="m first:6:alphabe")
        press(b"\x1b")
        press(b"\x06", expected="FIND")
        press(b"alpha", expected="m first:6:alpha")
        press(b"\r", expected="m first:15:alpha")
        press(b"\x1b[13;2u", expected="m first:6:alpha")
        press(b"\x1b", expected="m first:6:alpha")
        press(b"n", expected="m first:15:alpha")
        press(b"\x1bOR", expected="m second:0:alpha")
        press(b"\x1b[13;2~", expected="m first:15:alpha")
        press(b"\x06", expected="FIND")
        press(b"beta", expected="m second:6:beta")
        press(b"\x15", expected="Find in page")
        press(b"never-found", expected="not found")
        press(b"\x1b")
        press(b"\x0c", expected="OPEN")
        press(b"ac")
        press(b"\x1b[D")
        press(b"b", expected="ab")
        press(b"\x1b[H")
        press(b"\x1b[3~", expected="b")
        press(b"\x1b[F")
        press(b"d", expected="bcd")
        press(b"\x15", expected="Address or web search")
        press(b"one two")
        press(b"\x17", expected="one ")
        press(b"\x1b")
        press(b"q", expected="Ctrl-Q")
        press(b"?", expected="Webskiff keyboard help")
        press(b"G", expected="HELP MODE")
        press(b"g", expected="Webskiff keyboard help")
        press(b"?", expected="BROWSE")
        press(b"\x1bOP", expected="Webskiff keyboard help")
        press(b"\x1b")
        press(b"\x1b[15~", expected="keyboard fixture")
        press(b"\t", expected="focus button")
        press(b"\r", expected="button activated")
        press(b"\t", expected="focus entry")
        press(b"hello", expected="input:hello")
        press(b"?", expected="input:hello?")
        press(b"\x1bOP", expected="Webskiff keyboard help")
        press(b"\x1b")
        press(b"\x06", expected="FIND")
        press(b"\x1b")
        press(b"\x1b")
        press(b"\x14", expected="NEW TAB")
        press(fixture.encode() + b"\r", expected="2")
        press(b"\x1b[9;6u", expected="\x1b[7m 1")
        press(b"\x1b[9;5u", expected="\x1b[7m 2")
        press(b"\x17", expected="keyboard fixture")
        open_titled_tab("Zebra")
        assert active_tab(open_titled_tab("Quokka")) == 3
        active = pick_tab(3, "zbr", 2)
        pick_tab(active, "", active, cancel=True)
        # Enter with no text keeps the current tab.
        pick_tab(active, "", active)
        picker_survives_resize(active, 3)
        press(b"x")
        press(b"x", expected="\x1b[7m 1")
        pick_retitled_tab()
        print("KEYBOARD CHECKS PASSED")
        if args.check_only:
            # g e on a password field: refused, the editor does not run.
            press(b"gi", expected="focus entry")
            press(b"\t", expected="focus pw:secret")
            press(b"\x1b")
            external_edit("ok", "password field: not edited")
            assert not (editor_dir / "ran").exists(), "editor ran for a password field"
            press(b"gi", expected="focus entry")
            press(b"\t", expected="focus pw:secret")
            press(b"\x1b")
            # g e on a read-only field: refused, the editor does not run.
            press(b"gi", expected="focus entry")
            press(b"\t", expected="focus pw:secret")
            press(b"\t", expected="focus ro:fixed")
            press(b"\x1b")
            external_edit("ok", "read-only field: not edited")
            assert not (editor_dir / "ran").exists(), "editor ran for a read-only field"
            press(b"gi", expected="focus entry")
            press(b"\t", expected="focus pw:secret")
            press(b"\t", expected="focus ro:fixed")
            press(b"\x1b")
            # g e on a field in a disabled fieldset: refused. Leaving #off
            # disables its fieldset; a disabled field cannot take focus first.
            press(b"gi", expected="focus entry")
            press(b"\t\t\t", expected="focus lg off:kept")
            press(b"\t", expected="focus off")
            press(b"\x1b")
            external_edit("ok", "read-only field: not edited")
            assert not (editor_dir / "ran").exists(), "editor ran for a disabled field"
            # A field in the disabled fieldset's first legend is enabled: edited.
            press(b"gi", expected="focus entry")
            press(b"\t\t\t", expected="focus lg off:kept")
            press(b"\x1b")
            external_edit("ok", "legend:hello-ext")
            press(b"\x1b")
            # The editor fails: no write-back, the edit file is deleted.
            press(b"gi", expected="focus entry")
            press(b"\x1b")
            external_edit("fail", "editor failed")
            edit_file = Path((editor_dir / "path").read_text())
            assert edit_file.parent == edit_tmp, f"edit file not in TMPDIR: {edit_file}"
            assert not edit_file.exists(), f"edit file left: {edit_file}"
            assert not list(edit_tmp.glob("zweb-edit-*")), "zweb edit file left"
            press(b"gi")
            press(b"!", expected="input:hello?!")
            press(b"\x1b")
            # g e twice in one write: one editor; its keys stay out of the browser.
            (editor_dir / "mode").write_text("wait")
            press(b"gege")
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and not (editor_dir / "log").exists():
                pump(0.05)
            os.write(fd, b"?\r")
            output = bytearray()
            while time.monotonic() < deadline and "end" not in (editor_dir / "log").read_text():
                output.extend(b"".join(data for _, data in pump(0.05)))
            output.extend(b"".join(data for _, data in pump(1.5)))
            log = (editor_dir / "log").read_text().split()
            assert log == ["start", "end"], f"editor runs: {log}"
            assert b"no change" in output, f"no status after the editor: {bytes(output[-3000:])!r}"
            assert (editor_dir / "keys").read_text() == "?", "editor did not get its keys"
            assert b"Webskiff keyboard help" not in output, "browser took the editor's keys"
            # Success: the text comes back, the terminal is restored.
            output = external_edit("ok", "input:hello-ext")
            assert b"\x1b[?1049h" in output, "alternate screen not entered again"
            assert not list(edit_tmp.glob("zweb-edit-*")), "zweb edit file left"
            os.write(fd, b"\x11")
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                pump(0.1)
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    break
            assert done, "browser did not quit"
            pid = 0
            assert os.waitstatus_to_exitcode(status) == 0, f"exit status {status}"
            print("EXTERNAL EDITOR CHECKS PASSED")
            raise SystemExit(0)

    # Measure without title-changing test instrumentation or leftover form values.
    press(b"\x1b")
    press(b"o")
    press((fixture + "?measure").encode() + b"\r")
    pump(1)
    press(b"g")
    press(b"g")
    for _ in range(12):
        press(b"j", "scroll down")
        press(b"k", "scroll up")
    for _ in range(6):
        press(b"?", "help open")
        press(b"\x1b", "help close")
    for _ in range(12):
        press(b"g", "prefix g")
        press(b"\x1b")
    press(b"g")
    press(b"i")
    for character in "hello world":
        press(character.encode(), "page typing")
    press(b"\x01")
    press(b"\x7f")
    press(b"\x1b")
    press(b"t")
    press((fixture + "?measure&tab=2").encode() + b"\r")
    for _ in range(8):
        press(b"K", "tab switch")
    press(b"x")
    for _ in range(8):
        press(b"o", "open prompt")
        press(b"example", "prompt typing")
        press(b"\x1b")
    for _ in range(8):
        press(b"/")
        for character in "alpha":
            press(character.encode(), "find typing")
        press(b"\r", "find enter")
        press(b"n", "find next")
        press(b"N", "find previous")
    press(b"\x1b")
    open_titled_tab("Zebra")
    active = active_tab(open_titled_tab("Quokka"))
    for _ in range(8):
        query, target = ("zbr", 2) if active == 3 else ("qkk", 3)
        active = pick_tab(active, query, target, timed=True)
        pick_tab(active, "", active, cancel=True, timed=True)
    results = {}
    for label, rows in samples.items():
        results[label] = {
            metric: {f"p{int(p * 100)}": percentile([row[metric] for row in rows], p)
                     for p in (0.5, 0.9)}
            for metric in rows[0]
        }
    print(json.dumps(results, indent=2))
except AssertionError:
    log = Path(home.name) / ".cache/zweb/zweb.log"
    if log.exists():
        lines = [line for line in log.read_bytes().splitlines() if b"XPC error" not in line]
        print("BROWSER LOG:", repr(b"\n".join(lines)[-8000:]), flush=True)
    raise
finally:
    try:
        if pid and os.waitpid(pid, os.WNOHANG)[0] == 0:
            os.write(fd, b"\x11")
            pump(2)
            if os.waitpid(pid, os.WNOHANG)[0] == 0:
                os.kill(pid, signal.SIGTERM)
                os.waitpid(pid, 0)
    finally:
        os.close(fd)
        home.cleanup()
