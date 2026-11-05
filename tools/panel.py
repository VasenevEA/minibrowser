#!/usr/bin/env python3
"""Talks to a panel over its serial console (see serial_console.py).

A data collector that shares the port (any process named collect.py) is
paused (SIGSTOP) for the duration and resumed afterwards.
  panel.py put <local> <remote>     upload a file (chmod +x)
  panel.py run '<shell cmd>'        run a command, print output
"""
import os, signal, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from serial_console import Console


def collector_pids():
    out = subprocess.run(["pgrep", "-f", "collect.py"], capture_output=True, text=True).stdout
    return [int(p) for p in out.split()]


def main():
    pids = collector_pids()
    for p in pids:
        os.kill(p, signal.SIGSTOP)
    try:
        c = Console()
        c.shell()
        if sys.argv[1] == "put":
            t = time.time()
            c.put(open(sys.argv[2], "rb").read(), sys.argv[3])
            print(c.run(f"chmod +x {sys.argv[3]}; ls -la {sys.argv[3]}"), f"\n[{time.time() - t:.0f}s]")
        else:
            print(c.run(sys.argv[2], float(os.environ.get("T", "30"))))
        c.close()
    finally:
        for p in pids:
            os.kill(p, signal.SIGCONT)


if __name__ == "__main__":
    main()
