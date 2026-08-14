import os, pty, time, sys, select, fcntl, termios, struct

cmd = sys.argv[1:]
# A caller may separate its own arguments from the command to run with a bare "--", the
# way most CLIs read it. Strip one if present so cmd[0] is still the program to exec.
if cmd and cmd[0] == "--":
    cmd = cmd[1:]
# Batches separated by ~~ are sent with a pause between, so a key meant for a list
# that has not opened yet is not typed into the prompt behind it.
batches = [b for b in os.environ.get("KEYS", "").split("~~") if b != ""]
wait = float(os.environ.get("WAIT", "3.0"))
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execvp(cmd[0], cmd)

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))

out = b""
seen = b""
start = time.time()
deadline = start + wait
# Batches go out on a clock rather than when the stream goes quiet: a TUI that ticks is
# never quiet, and a key meant for a list that has not opened yet lands in the prompt.
gap = float(os.environ.get("GAP", "1.5"))
due = start + gap
while time.time() < deadline:
    if batches and time.time() >= due:
        os.write(fd, batches.pop(0).encode())
        due = time.time() + gap
    r, _, _ = select.select([fd], [], [], 0.2)
    if r:
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
        seen += chunk
        # Answer what a real terminal answers, or the program waits for it.
        if b"\x1b[6n" in seen:
            seen = seen.replace(b"\x1b[6n", b"")
            os.write(fd, b"\x1b[1;1R")
        if b"\x1b]11;?" in seen:
            seen = seen.replace(b"\x1b]11;?", b"")
            os.write(fd, b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\")

try:
    os.close(fd)
except OSError:
    pass
sys.stdout.buffer.write(out)
