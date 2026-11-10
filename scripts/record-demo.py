"""Record real Webskiff output against a public, local fixture as asciicast v2."""

import argparse
import codecs
import fcntl
import gzip
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import struct
import tempfile
import termios
import time


ROOT = Path(__file__).resolve().parents[1]


def record(binary, output):
    with tempfile.TemporaryDirectory(prefix="webskiff-demo-", dir="/tmp") as directory:
        directory = Path(directory)
        shutil.copy2(ROOT / "bench/keyboard.html", directory / "demo.html")
        address = (directory / "demo.html").as_uri() + "?demo=1"
        home = directory / "home"
        home.mkdir()
        start = time.monotonic()
        events = []
        pid, fd = pty.fork()
        if pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 1920, 1280))
            os.environ["HOME"] = str(home)
            os.environ.pop("ZWEB_TRACE", None)
            os.environ.pop("ZWEB_DEBUG", None)
            os.execv(binary, [binary, "--gfx", "kitty", address])
        try:
            run_demo(fd, start, events, address)
            exited, status = os.waitpid(pid, os.WNOHANG)
            if not exited:
                raise RuntimeError("browser did not shut down after quit")
            pid = None
            if status != 0:
                raise RuntimeError(f"browser exited with status {status}")
        finally:
            os.close(fd)
            if pid is not None:
                os.kill(pid, signal.SIGTERM)
                os.waitpid(pid, 0)
        header = {
            "version": 2, "width": 120, "height": 40,
            "title": "Webskiff: real local-fixture keyboard session",
            "env": {"TERM": "xterm-256color"},
        }
        data = "\n".join(json.dumps(item) for item in [header, *events]) + "\n"
        output.parent.mkdir(parents=True, exist_ok=True)
        with output.open("xb") as stream:
            stream.write(gzip.compress(data.encode(), mtime=0))
        print(f"Recorded {len(events)} output events to {output}")


def run_demo(fd, start, events, address):
    decoder = codecs.getincrementaldecoder("utf-8")()

    def pump(seconds):
        deadline = time.monotonic() + seconds
        raw = bytearray()
        while time.monotonic() < deadline:
            if select.select([fd], [], [], min(0.02, max(0, deadline - time.monotonic())))[0]:
                try:
                    chunk = os.read(fd, 1 << 20)
                except OSError:
                    break
                if not chunk:
                    break
                raw.extend(chunk)
                text = decoder.decode(chunk)
                if text:
                    events.append([round(time.monotonic() - start, 6), "o", text])
        return raw

    pump(0.5)
    os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c")
    ready = pump(5)
    if b"keyboard fixture" not in ready:
        raise RuntimeError("local demo fixture did not load")
    steps = [
        (b"jjj", 1.0), (b"gg", 1.0), (b"/", 0.4),
        (b"Browser", 1.0), (b"\r", 0.8), (b"n", 0.8),
        (b"N", 0.8), (b"t", 0.5), (address.encode(), 0.5),
        (b"\r", 1.0), (b"[", 0.7), (b"f", 1.0),
        (b"\x1b", 0.5), (b"?", 2.0), (b"\x1b", 0.5),
        (b"\x11", 1.0),
    ]
    for keys, seconds in steps:
        os.write(fd, keys)
        pump(seconds)
    raw = "".join(event[2] for event in events)
    if " FIND " not in raw or "Webskiff keyboard help" not in raw:
        raise RuntimeError("recording did not exercise find and help")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    record(str(args.binary.resolve()), args.output)


if __name__ == "__main__":
    main()
