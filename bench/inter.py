# Times key -> first page pixels written, for several kinds of interaction.
import os, pty, sys, time, struct, fcntl, termios, select, re
B = sys.argv[1]
log = os.path.expanduser("~/.cache/zweb/zweb.log"); open(log, "w").close()
html = "data:text/html,<title>form</title><body style='font:18px sans-serif'><input id=q style='font-size:20px;width:600px' autofocus><p>" + "<a href=%23>link</a> words " * 200
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 120*16, 40*32))
    os.environ["ZWEB_TRACE"] = "1"
    os.execv(B, [B, "--gfx", "kitty", "file://"+os.path.abspath("static.html")+""])
def pump(s):
    end = time.time() + s
    while time.time() < end:
        if select.select([fd], [], [], 0.005)[0]:
            try: os.read(fd, 1 << 20)
            except OSError: return
pump(0.5); os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c"); pump(5)
marks = []
def act(label, keys, wait=0.4):
    marks.append((label, time.time())); os.write(fd, keys); pump(wait)
for _ in range(8): act("space (page down)", b" ")
for _ in range(4): act("b (page up)", b"b")
act("open tab 2", b"t"); pump(0.2); os.write(fd, html.encode() + b"\r"); pump(3)
os.write(fd, b"gi"); pump(0.5)
for c in "hello world":  act("type a char", c.encode(), 0.25)
os.write(fd, b"\x1b"); pump(0.3)
for _ in range(5):
    act("f (show hints)", b"f", 0.5); os.write(fd, b"\x1b"); pump(0.4)
for _ in range(6): act("switch tab", b"K", 0.5)
os.write(fd, b"\x11"); pump(2)
# Trace times are ms since zweb start; align by the first "input read" of each mark.
ev = [(float(m[1]), m[2]) for m in re.finditer(r"^T +([\d.]+) (.*)$", open(log).read(), re.M)]
reads = [t for t, e in ev if e.startswith("input read Key")]
out = [t for t, e in ev if e in ("frame written", "hints written") or e.startswith("predict written")]
# Map each mark to the input read of its first key, in order.
res = {}
ri = 0; t0w = time.time()
key_reads = iter(reads)
# Count keys sent per action precisely: marks were single keys except where noted.
for (label, _), tr in zip(marks, [r for r in reads]):
    pass
# Simpler: match marks to reads by order of single-key actions only.
singles = [m for m in marks]
# Reads include extra keys (t, URL chars, gi, Esc); find them by event text instead.
evk = [(t, e) for t, e in ev if e.startswith("input read Key")]
def keyname(e):
    m = re.search(r"code: (Char\('(.+?)'\)|\w+)", e); return m[2] or m[1] if m else "?"
want = {"space (page down)": " ", "b (page up)": "b", "type a char": None, "f (show hints)": "f", "switch tab": "K"}
typed = iter("hello world")
idx = 0
typed_started = False
for label, _ in [m for m in marks if m[0] in want]:
    if label == "type a char" and not typed_started:
        typed_started = True
        while idx + 1 < len(evk) and not (keyname(evk[idx][1]) == "g" and keyname(evk[idx + 1][1]) == "i"): idx += 1
        idx += 2
    k = want[label] or next(typed)
    while idx < len(evk) and keyname(evk[idx][1]) != k: idx += 1
    if idx >= len(evk): break
    t = evk[idx][0]; idx += 1
    after = [o for o in out if o > t]
    if after and after[0] - t < 300: res.setdefault(label, []).append(after[0] - t)
q = lambda v, p: sorted(v)[min(len(v)-1, int(p*len(v)))]
for label, v in res.items():
    print(f"{label:20} p50 {q(v,.5):6.1f}  p90 {q(v,.9):6.1f}  max {max(v):6.1f} ms  (n={len(v)})")
