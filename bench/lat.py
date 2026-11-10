# Runs zweb with ZWEB_TRACE, presses j N times, reports wheel->frame and key->written.
import os, pty, sys, time, struct, fcntl, termios, select, re, statistics as st
B, url = sys.argv[1], sys.argv[2]; n = int(sys.argv[3]) if len(sys.argv) > 3 else 20
log = os.path.expanduser("~/.cache/zweb/zweb.log"); open(log, "w").close()
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 120*16, 40*32))
    os.environ["ZWEB_TRACE"] = "1"
    os.execv(B, [B, "--gfx", "kitty", url])
def pump(s):
    end = time.time() + s
    while time.time() < end:
        if select.select([fd], [], [], 0.01)[0]:
            try: os.read(fd, 1 << 20)
            except OSError: return
pump(0.5); os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c"); pump(6)
seq = (os.environ.get("KEYS") or "j" * n)
for ch in seq:
    os.write(fd, ch.encode()); pump(0.3)
os.write(fd, b"\x11"); pump(2)
ev = [(float(m[1]), m[2]) for m in re.finditer(r"^T +([\d.]+) (.*)$", open(log).read(), re.M)]
w2f, k2w, k2p = [], [], []
for i, (t, e) in enumerate(ev):
    if e.startswith("input read Key"):
        rest = [x for x in ev[i+1:] if x[0] - t < 240]
        nxt = next((j for j, (t2, e2) in enumerate(rest) if e2.startswith("input read")), len(rest))
        rest = rest[:nxt]
        sent = next((t2 for t2, e2 in rest if e2.startswith("wheel sent")), None)
        pw = next((t2 for t2, e2 in rest if e2.startswith("predict written")), None)
        if pw: k2p.append(pw - t)
        if os.environ.get("SHOW"): print(" ", re.search(r"Char\('(.)'\)", e)[1], "predict" if pw else "frame  ", round((pw or 0) - t, 1) if pw else "")
        # The scroll's frame: the first encoded one that reused moved rows.
        idx = next((j for j, (t2, e2) in enumerate(rest) if (m := re.match(r"frame encoded \d+ new (\d+) moved", e2)) and (int(m[1]) >= 5 or pw)), None)
        if sent is None or idx is None: continue
        recvs = [t2 for t2, e2 in rest[:idx] if "recv" in e2]
        if not recvs: continue
        recv = max(recvs)
        wr = next(t2 for t2, e2 in rest[idx:] if e2 == "frame written")
        w2f.append(recv - sent); k2w.append(wr - t)
        if wr - t > 12 and os.environ.get("SHOW"):
            print("SLOW", round(wr - t, 1), [(round(t2 - t, 1), e2[:70]) for t2, e2 in rest[:idx + 2]])
q = lambda v, p: sorted(v)[min(len(v)-1, int(p*len(v)))] if v else float("nan")
print(f"wheel->frame  p50 {q(w2f,.5):6.1f}  p90 {q(w2f,.9):6.1f}  max {max(w2f or [0]):6.1f} ms  (n={len(w2f)})")
print(f"key->written  p50 {q(k2w,.5):6.1f}  p90 {q(k2w,.9):6.1f}  max {max(k2w or [0]):6.1f} ms  (n={len(k2w)})")
print(f"key->predict  p50 {q(k2p,.5):6.1f}  p90 {q(k2p,.9):6.1f}  max {max(k2p or [0]):6.1f} ms  (n={len(k2p)} of {len(k2w)})")
settled = re.findall(r"settled best (-?\d+) of (-?\d+), score (\d+) of (\d+)", open(log).read())
print("settles:", len(settled), "exact:", sum(a == b for a, b, *_ in settled), "low score:", [x for x in settled if int(x[2]) * 3 < int(x[3]) * 2][:5])
