"""Measure decoded video and terminal frame delivery without network buffering.

Requires ffmpeg. The PTY consumes Kitty transfers, including shared-memory
objects, but does not measure a real terminal's image upload or display time.
"""

import argparse
import base64
import ctypes
import fcntl
import json
import mmap
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time


def percentile(values, fraction):
    values = sorted(values)
    return round(values[min(int(len(values) * fraction), len(values) - 1)], 2) if values else None


def report(args, trace, elapsed, count, transfers, shared_bytes, initial, final):
    encoded = [
        (float(match[1]), int(match[2]), float(match[3]))
        for match in re.finditer(
            r"^T +([\d.]+) frame encoded (\d+) new \d+ moved \d+ KB in ([\d.]+) ms",
            trace, re.M,
        )
    ]
    written = [float(match[1]) for match in re.finditer(r"^T +([\d.]+) frame written$", trace, re.M)]
    intervals = [b - a for a, b in zip(written, written[1:])]
    result = {
        "gfx": args.gfx,
        "source_fps": args.fps,
        "seconds": round(elapsed, 2),
        "delivered_fps": round(len(written) / elapsed, 2),
        "frame_interval_ms_p50_p90": [percentile(intervals, p) for p in (0.5, 0.9)],
        "encode_ms_p50_p90": [percentile([e[2] for e in encoded], p) for p in (0.5, 0.9)],
        "new_tiles_p50_p90": [percentile([e[1] for e in encoded], p) for p in (0.5, 0.9)],
        "terminal_KiB_per_second": round(count / elapsed / 1024, 2),
        "image_transfers_per_second": round(transfers / elapsed, 2),
        "shared_MiB_per_second": round(shared_bytes / elapsed / 1024**2, 2),
        "video_total_frames": final[1] - initial[1],
        "dropped_frames": final[2] - initial[2],
        "video_callbacks": final[3] - initial[3],
    }
    print(json.dumps(result, indent=2))
    if args.check:
        assert result["delivered_fps"] >= args.fps * 0.9, "terminal delivery below 90% of source rate"
        assert result["frame_interval_ms_p50_p90"][1] <= 1500 / args.fps, "uneven terminal frame delivery"
        assert result["dropped_frames"] == 0, "Chromium dropped video frames"


def measure(args, home):
    binary = str(Path(args.binary).resolve())
    video = home / "video.webm"
    subprocess.run(
        [
            "ffmpeg", "-v", "error", "-f", "lavfi", "-i",
            f"testsrc2=size=960x540:rate={args.fps}", "-t", "20",
            "-c:v", "libvpx", "-deadline", "realtime", "-cpu-used", "8",
            "-b:v", "2M", "-an", str(video),
        ],
        check=True,
    )
    page = home / "video.html"
    page.write_text("""<!doctype html>
<html lang="en"><meta charset="utf-8"><title>video loading</title>
<body style="margin:0;background:#f0f2f3;color:#24292f">
<video aria-label="Test pattern" src="video.webm" autoplay muted loop playsinline
  style="width:100%;height:auto"></video>
<script>
const video = document.querySelector('video');
let callbacks = 0;
function frame() { callbacks++; video.requestVideoFrameCallback(frame); }
video.requestVideoFrameCallback(frame);
setInterval(() => {
  const q = video.getVideoPlaybackQuality();
  document.title = `V${video.currentTime.toFixed(1)}/${q.totalVideoFrames}/${q.droppedVideoFrames}/${callbacks}!`;
}, 1000);
</script></body></html>""")
    pid, fd = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 1920, 1280))
        os.environ["HOME"] = str(home)
        os.environ["ZWEB_TRACE"] = "1"
        os.execv(binary, [binary, "--gfx", args.gfx, page.as_uri()])

    libc = ctypes.CDLL(None, use_errno=True)
    libc.shm_open.argtypes = [ctypes.c_char_p, ctypes.c_int, ctypes.c_uint]
    libc.shm_unlink.argtypes = [ctypes.c_char_p]
    pending = bytearray()
    titles = []
    title_tail = b""
    transfers = 0
    shared_bytes = 0

    def consume_shared(payload, size):
        name = base64.b64decode(payload)
        shared_fd = libc.shm_open(name, os.O_RDONLY, 0)
        if shared_fd < 0:
            raise OSError(ctypes.get_errno(), f"cannot read image {name!r}")
        try:
            with mmap.mmap(shared_fd, size, access=mmap.ACCESS_READ) as image:
                # Read every page, as a terminal would, before unlinking.
                sum(image[::4096])
        finally:
            os.close(shared_fd)
            if libc.shm_unlink(name) != 0:
                raise OSError(ctypes.get_errno(), f"cannot unlink image {name!r}")
        return size

    def consume(data):
        nonlocal pending, title_tail, transfers, shared_bytes
        title_tail += data
        for match in re.finditer(rb"V([\d.]+)/(\d+)/(\d+)/(\d+)!", title_tail):
            titles.append([float(match[1]), *map(int, match.groups()[1:])])
        title_tail = title_tail[-100:]
        pending.extend(data)
        while True:
            start = pending.find(b"\x1b_G")
            if start < 0:
                pending = pending[-3:]
                return
            end = pending.find(b"\x1b\\", start)
            if end < 0:
                del pending[:start]
                return
            command = bytes(pending[start + 3:end])
            del pending[:end + 2]
            head, _, payload = command.partition(b";")
            fields = dict(item.split(b"=", 1) for item in head.split(b",") if b"=" in item)
            if fields.get(b"a") in (b"T", b"t"):
                transfers += 1
            if fields.get(b"t") == b"s":
                shared_bytes += consume_shared(payload, int(fields[b"S"]))

    def pump(seconds):
        count = 0
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([fd], [], [], 0.005)[0]:
                try:
                    data = os.read(fd, 1 << 20)
                except OSError:
                    break
                count += len(data)
                consume(data)
        return count

    try:
        pump(0.3)
        os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\\x1b[?62;c")
        log = home / ".cache/zweb/zweb.log"
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and (not titles or titles[-1][0] < 2):
            pump(0.25)
        assert titles and titles[-1][0] > 1, f"video did not play: {titles}\n{log.read_text()[-5000:]}"
        initial = titles[-1]
        offset = log.stat().st_size
        transfers = shared_bytes = 0
        start = time.monotonic()
        count = pump(args.seconds)
        elapsed = time.monotonic() - start
        report(args, log.read_text()[offset:], elapsed, count, transfers, shared_bytes, initial, titles[-1])
    finally:
        os.write(fd, b"\x11")
        pump(2)
        if os.waitpid(pid, os.WNOHANG)[0] == 0:
            os.kill(pid, signal.SIGTERM)
            os.waitpid(pid, 0)
        os.close(fd)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("--gfx", choices=["kitty", "shm"], default="shm")
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--seconds", type=float, default=8)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="webskiff-video-") as home:
        measure(args, Path(home))
