"""Runs a command in a pseudo-terminal and plays a scripted dialogue with it.

Used by the CLI's terminal tests (`pty.ts`). Reads a JSON spec on stdin:

    {"argv": [...], "env": {...}, "cwd": "...", "timeout": seconds,
     "steps": [{"expect": "Password: ", "send": "secret\\r"}, ...]}

Each step waits until `expect` appears in the terminal output (after the
previous step's match) and only then writes `send`: characters typed before
the program switches the terminal to raw mode would be echoed by the terminal
driver. After the program has exited, `stty -a` runs on the same terminal.

Prints one JSON object on stdout: {"exit": code (negative: signal),
"output": base64 of everything the terminal showed, "stty": "..."}, or
{"error": "...", "output": ...} when a step or the exit timed out (the program
is then killed).
"""

import base64
import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time


def main():
    spec = json.load(sys.stdin)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))

    def controlling_terminal():
        # After setsid (start_new_session): the terminal becomes the session's
        # controlling terminal, as in an interactive shell.
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    proc = subprocess.Popen(
        spec["argv"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=spec["env"],
        cwd=spec["cwd"],
        start_new_session=True,
        preexec_fn=controlling_terminal,
    )
    output = bytearray()
    deadline = time.monotonic() + spec["timeout"]

    def read_some(wait):
        ready, _, _ = select.select([master], [], [], max(wait, 0))
        if not ready:
            return False
        try:
            data = os.read(master, 65536)
        except OSError:
            return False
        output.extend(data)
        return bool(data)

    def result(**fields):
        fields["output"] = base64.b64encode(bytes(output)).decode("ascii")
        json.dump(fields, sys.stdout)

    def fail(message):
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        while read_some(0):
            pass
        result(error=message)
        sys.exit(0)

    position = 0
    for step in spec["steps"]:
        expected = step["expect"].encode("utf-8")
        while True:
            index = output.find(expected, position)
            if index >= 0:
                position = index + len(expected)
                break
            if proc.poll() is not None and not read_some(0.05):
                fail(f"the program exited (code {proc.returncode}) before showing {step['expect']!r}")
            if time.monotonic() > deadline:
                fail(f"timed out waiting for {step['expect']!r}")
            read_some(min(deadline - time.monotonic(), 0.1))
        os.write(master, step["send"].encode("utf-8"))

    while proc.poll() is None:
        if time.monotonic() > deadline:
            fail("timed out waiting for the program to exit")
        read_some(min(deadline - time.monotonic(), 0.1))
    while read_some(0.05):
        pass

    stty = subprocess.run(["stty", "-a"], stdin=slave, capture_output=True, text=True)
    result(exit=proc.returncode, stty=stty.stdout + stty.stderr)


if __name__ == "__main__":
    main()
