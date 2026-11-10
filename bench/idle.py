# Idle cost: bytes/s and CPU of zweb + its helpers over 5 s, after load.
import os, pty, sys, time, struct, fcntl, termios, select, subprocess
B, url = sys.argv[1], sys.argv[2]
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 120*16, 40*32))
    os.execv(B, [B, "--gfx", "kitty", url])
def pump(s):
    n = 0; end = time.time() + s
    while time.time() < end:
        if select.select([fd], [], [], 0.01)[0]:
            try: n += len(os.read(fd, 1 << 20))
            except OSError: break
    return n
def cpu():
    out = subprocess.run(["ps", "-axo", "ppid=,pid=,time="], capture_output=True, text=True).stdout
    procs = [l.split() for l in out.splitlines()]
    kids = {pid}; changed = True
    while changed:
        changed = False
        for pp, p, t in procs:
            if int(pp) in kids and int(p) not in kids: kids.add(int(p)); changed = True
    tot = 0.0
    for pp, p, t in procs:
        if int(p) in kids:
            m, s = t.split(":"); tot += int(m) * 60 + float(s)
    return tot
pump(0.5); os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c"); pump(6)
c0 = cpu(); n = pump(5); c1 = cpu()
print(f"idle: {n/5/1024:7.1f} KB/s to terminal, CPU {100*(c1-c0)/5:5.1f}% (all processes)")
os.write(fd, b"\x11"); pump(2)
