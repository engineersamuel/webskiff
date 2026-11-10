import os, pty, sys, time, struct, fcntl, termios, select, subprocess
B, url = sys.argv[1], sys.argv[2]; X = sys.argv[3:]
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 120*16, 40*32))
    os.execv(B, [B, "--gfx", "kitty", *X, url])
def pump(s):
    n = 0; end = time.time() + s
    while time.time() < end:
        if select.select([fd], [], [], 0.002)[0]:
            try: n += len(os.read(fd, 1 << 20))
            except OSError: break
    return n
def cpu():
    out = subprocess.run(["ps", "-axo", "ppid=,pid=,time="], capture_output=True, text=True).stdout
    procs = [l.split() for l in out.splitlines()]; kids = {pid}; ch = True
    while ch:
        ch = False
        for pp, p, t in procs:
            if int(pp) in kids and int(p) not in kids: kids.add(int(p)); ch = True
    return sum(int(t.split(":")[0]) * 60 + float(t.split(":")[1]) for pp, p, t in procs if int(p) in kids)
pump(0.5); os.write(fd, b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c"); pump(6)
c0 = cpu(); n = 0; t0 = time.time()
for i in range(90):  # 3 s of a held key at 30/s, down then up
    os.write(fd, b"j" if i < 45 else b"k"); n += pump(1 / 30)
c1 = cpu(); el = time.time() - t0
print(f"held j/k 3 s: CPU {100*(c1-c0)/el:5.1f}%, {n/el/1024:6.1f} KB/s to terminal")
os.write(fd, b"\x11"); pump(2)
