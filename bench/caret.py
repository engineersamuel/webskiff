# Times visual-mode keys (v, then l, j, w), key -> first page pixels written.
# Usage: caret.py <webskiff binary> [url] [count]
import os, pty, sys, time, struct, fcntl, termios, select, re
B = sys.argv[1]
url = sys.argv[2] if len(sys.argv) > 2 else "https://en.wikipedia.org/wiki/Cat"
n = int(sys.argv[3]) if len(sys.argv) > 3 else 30
log = os.path.expanduser("~/.cache/zweb/zweb.log"); open(log, "w").close()
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 120*16, 40*32))
    os.environ["ZWEB_TRACE"] = "1"
    os.execv(B, [B, "--gfx", "kitty", url])
def pump(s):
    end = time.time() + s
    while time.time() < end:
        if select.select([fd], [], [], 0.005)[0]:
            try: os.read(fd, 1 << 20)
            except OSError: return
pump(0.5); os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c"); pump(8)
os.write(fd, b"v"); pump(1)
keys = "l" * n + "j" * n + "w" * n
for c in keys:
    os.write(fd, c.encode()); pump(0.25)
os.write(fd, b"y"); pump(0.5)
os.write(fd, b"\x11"); pump(2)
ev = [(float(m[1]), m[2]) for m in re.finditer(r"^T +([\d.]+) (.*)$", open(log).read(), re.M)]
out = [t for t, e in ev if e == "frame written"]
def keyname(e):
    m = re.search(r"code: (Char\('(.+?)'\)|\w+)", e); return m[2] or m[1] if m else "?"
reads = [(t, keyname(e)) for t, e in ev if e.startswith("input read Key")]
# Skip reads up to the v that entered visual mode, then pair keys in order.
i = next(i for i, (_, k) in enumerate(reads) if k == "v") + 1
res, missed = {}, {}
for want in keys:
    while i < len(reads) and reads[i][1] != want: i += 1
    if i >= len(reads): break
    t = reads[i][0]; i += 1
    after = [o for o in out if o > t]
    if after and after[0] - t < 300: res.setdefault(want, []).append(after[0] - t)
    else: missed[want] = missed.get(want, 0) + 1
q = lambda v, p: sorted(v)[min(len(v)-1, int(p*len(v)))]
for k in "ljw":
    v = res.get(k, [])
    if v: print(f"visual {k:2} p50 {q(v,.5):6.1f}  p90 {q(v,.9):6.1f}  max {max(v):6.1f} ms  (n={len(v)}, no frame: {missed.get(k, 0)})  vs 10 ms: p90 {'OK' if q(v,.9) <= 10 else 'OVER'}")
    else: print(f"visual {k:2} no samples (no frame: {missed.get(k, 0)})")
