#!/usr/bin/env python3
"""Exercise the real terminal backend using only synthetic --demo data."""
import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time

binary = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/debug/agent-flow")
pid, master = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 160, 0, 0))
    os.environ["TERM"] = "xterm-256color"
    os.execv(binary, [binary, "--demo"])

captured = bytearray()


def drain(seconds=0.25):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        ready, _, _ = select.select([master], [], [], 0.05)
        if ready:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                return
            if not chunk:
                return
            captured.extend(chunk)


try:
    drain(1)
    assert b"AGENT FLOW" in captured, "initial frame missing"
    assert b"DASHBOARD" in captured, "dashboard missing"
    assert b"2 TURNS" in captured, "turn list missing"
    captured.clear()
    os.write(master, b"x")
    drain()
    assert b"Baseline tests passed" in captured, "slowest tool drill-down missing"
    captured.clear()
    os.write(master, b"vff")
    drain()
    assert b"PARALLEL" in captured, "family lanes missing"
    captured.clear()
    os.write(master, b"lgj\r")
    drain()
    assert b"FAILED retry_limit" in captured, "lane drill-down did not open the selected tool"
    os.write(master, b"b")
    drain()
    for keys in [b"d", b"ooo", b"\r", b"d", b"2j", b"\r"]:
        os.write(master, keys)
        drain()
    for keys in [b"2g", b"jjj\r", b"b", b"3", b"j]", b"/retry\r", b"\x1b", b"?", b"jj", b"\x1b", b" ", b" "]:
        os.write(master, keys)
        drain()
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    drain()
    os.write(master, b"d2j1d")
    drain()
    os.write(master, b"1p2t3")
    drain()
    os.write(master, b"vhljg\r")
    drain()
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 12, 42, 0, 0))
    drain()
    os.write(master, b"vlh\r")
    drain()
    os.write(master, b"q")
    drain(0.5)
    deadline = time.monotonic() + 5
    status = None
    while time.monotonic() < deadline:
        result, value = os.waitpid(pid, os.WNOHANG)
        if result:
            status = value
            break
        time.sleep(0.05)
    assert status is not None, "TUI did not exit"
    assert os.waitstatus_to_exitcode(status) == 0, f"exit status {status}"
    assert b"\x1b[?1049l" in captured, "alternate screen was not restored"
    assert b"panicked" not in captured, "panic detected"
    print("PTY smoke passed: dashboard, agent detail, family lanes, exact tool/turn drill-down, filters, follow, resize down to 42x12, clean exit")
finally:
    os.close(master)
    try:
        result, _ = os.waitpid(pid, os.WNOHANG)
        if not result:
            os.kill(pid, 15)
            os.waitpid(pid, 0)
    except ChildProcessError:
        pass
