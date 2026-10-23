#!/usr/bin/env python3
"""Clicks at (x, y) screen pixels in a QEMU VM through QMP (usb-tablet).
    qclick.py SOCKET X Y WIDTH HEIGHT"""
import json, socket, sys, time
path, x, y, w, h = sys.argv[1], *map(int, sys.argv[2:6])
s = socket.socket(socket.AF_UNIX); s.connect(path); f = s.makefile("rw"); f.readline()
def call(cmd, args=None):
    f.write(json.dumps({"execute": cmd, **({"arguments": args} if args else {})}) + "\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r: return r
call("qmp_capabilities")
move = [{"type": "abs", "data": {"axis": "x", "value": x * 32767 // w}}, {"type": "abs", "data": {"axis": "y", "value": y * 32767 // h}}]
call("input-send-event", {"events": move}); time.sleep(0.2)
for down in (True, False):
    call("input-send-event", {"events": [{"type": "btn", "data": {"down": down, "button": "left"}}]}); time.sleep(0.1)
