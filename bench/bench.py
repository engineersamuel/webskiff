# Drives zweb in a pty like a terminal would, and times what it writes.
import os, pty, sys, time, struct, fcntl, termios, select, statistics as st
B, url, gfx = sys.argv[1], sys.argv[2], sys.argv[3]
extra = sys.argv[4:]
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 120*16, 40*32))
    os.execv(B, [B, "--gfx", gfx, *extra, url])
log = []  # (time, bytes)
def pump(secs, quiet=None):
    """Read for `secs`, or until no output for `quiet` seconds."""
    end = time.time() + secs; last = time.time()
    while time.time() < end:
        if select.select([fd], [], [], 0.005)[0]:
            try: d = os.read(fd, 1 << 20)
            except OSError: return
            log.append((time.time(), d)); last = time.time()
        elif quiet and time.time() - last > quiet: return
def step(keys, gap=0.0):
    log.clear(); t0 = time.time()
    for k in keys:
        os.write(fd, k); pump(gap) if gap else None
    t_last = time.time()
    pump(3, quiet=0.25)
    imgs = [(t, d) for t, d in log if b"a=T" in d or b"a=p" in d]
    first = imgs[0][0] - t0 if imgs else float("nan")
    done = (imgs[-1][0] - t_last) if imgs else float("nan")
    total = sum(len(d) for _, d in log)
    n = sum(d.count(b"a=T") for _, d in log) + sum(d.count("▀".encode()) > 0 for _, d in log)
    return first * 1000, done * 1000, total, sum(d.count(b"a=T") for _, d in log), sum(d.count(b"a=p") for _, d in log)
# Answer the start-up query as Kitty does (graphics OK for direct, no shm), then load.
pump(0.5); os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c")
end = time.time() + 10
while time.time() < end and not any(b"a=T" in d for _, d in log): pump(0.1)
log.clear(); pump(6, quiet=1.0)
rows = {"single j": [], "single k": [], "held j x15 (33ms repeat)": [], "single space (page)": []}
for _ in range(5):
    rows["single j"].append(step([b"j"]))
for _ in range(5):
    rows["single k"].append(step([b"k"]))
for _ in range(3):
    rows["held j x15 (33ms repeat)"].append(step([b"j"] * 15, gap=0.033))
for _ in range(3):
    rows["single space (page)"].append(step([b" "]))
os.write(fd, b"\x11"); pump(2)
print(f"{'action':28} {'key->first img':>15} {'last key->last img':>19} {'B out':>8} {'sent':>5} {'moved':>6}")
for k, v in rows.items():
    med = lambda i: st.median(x[i] for x in v)
    print(f"{k:28} {med(0):12.0f} ms {med(1):16.0f} ms {med(2):8.0f} {med(3):5.0f} {med(4):6.0f}")
