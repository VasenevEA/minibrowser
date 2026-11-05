#!/usr/bin/env python3
"""Synthetic taps on the T113 panel: writes evdev events into its touchscreen.
  tap.py X,Y [X,Y ...]      screen pixels (800x480), one tap per pair, 0.6 s apart
Each tap is jittered by a raw unit or two: the kernel drops ABS events that repeat
the previous value."""
import base64, os, random, struct, subprocess, sys

def ev(t, c, v):
    return struct.pack("<IIHHi", 0, 0, t, c, v)

cmds = []
for arg in sys.argv[1:]:
    x, y = map(float, arg.split(","))
    rx = int(x / 800 * 4095) + random.choice((-2, -1, 1, 2))
    ry = int(y / 480 * 4095) + random.choice((-2, -1, 1, 2))
    b = ev(1, 330, 1) + ev(3, 0, rx) + ev(3, 1, ry) + ev(3, 24, 200) + ev(0, 0, 0) + ev(1, 330, 0) + ev(3, 24, 0) + ev(0, 0, 0)
    cmds.append(f"echo {base64.b64encode(b).decode()} | base64 -d > /dev/input/event0; sleep 0.6")
here = os.path.dirname(os.path.abspath(__file__))
subprocess.run([sys.executable, os.path.join(here, "panel.py"), "run", "; ".join(cmds) + "; tail -n %d /tmp/mb.log" % (len(cmds) + 1)],
               env={**os.environ, "T": "30"})
