#!/usr/bin/env python3
"""Render the TUI in a pty and print the final screen.

This is how the screens in the definition of done were produced: the app runs for real,
against fake data or a real cluster, and the terminal output is replayed through a VT
emulator so the text can be looked at and diffed.

    shot.py "<command>" [cols] [rows] [seconds] [keys]

`keys` is fed one key at a time with a pause after each, e.g. "uu" or "/petrova".

Needs pyte (`pip3 install --user pyte`). One caveat: pyte has no alternate-screen support,
so a row can show a leftover from the frame before it. `cargo test ui::tests::dump_screen`
prints a clean frame through ratatui's own TestBackend.
"""

import fcntl
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

import pyte


def main() -> int:
    if len(sys.argv) < 5:
        print(__doc__, file=sys.stderr)
        return 2
    cmd, cols, rows, seconds = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), float(sys.argv[4])
    keys = sys.argv[5] if len(sys.argv) > 5 else ""

    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["COLUMNS"], os.environ["LINES"] = str(cols), str(rows)
        # Size the pty before the app starts: otherwise its first frame is drawn at the
        # default size and the differential repaint leaves a mix of two layouts behind.
        os.execvp(
            "bash",
            ["bash", "-lc", f"stty rows {rows} cols {cols}; exec bash -lc {json.dumps(cmd)}"],
        )

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    screen = pyte.Screen(cols, rows)
    stream = pyte.Stream(screen)

    def pump(seconds: float) -> None:
        end = time.time() + seconds
        while time.time() < end:
            readable, _, _ = select.select([fd], [], [], 0.1)
            if not readable:
                continue
            try:
                data = os.read(fd, 65536)
            except OSError:
                return
            if not data:
                return
            stream.feed(data.decode("utf-8", "replace"))

    pump(seconds)
    for key in keys:
        os.write(fd, key.encode())
        pump(1.5)

    # pyte has no alternate-screen buffer, so the first frame's leftovers can bleed through a
    # later one. A resize makes ratatui clear and repaint everything, which pyte does apply.
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    os.kill(pid, signal.SIGWINCH)
    pump(1.0)

    print("\n".join(line.rstrip() for line in screen.display))

    os.write(fd, b"q")
    pump(0.5)
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())