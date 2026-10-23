#!/usr/bin/env python3
"""Types into a QEMU VM through its QMP socket (US layout), no window needed.

    qtype.py SOCKET text "sudo dnf install -y openssh-server"   # types, then Enter
    qtype.py SOCKET keys ctrl-alt-t                             # one key combination
"""
import json
import socket
import sys
import time

NAMED = {" ": "spc", "\n": "ret", "\t": "tab", "-": "minus", "=": "equal", "[": "bracket_left",
         "]": "bracket_right", ";": "semicolon", "'": "apostrophe", "`": "grave_accent",
         "\\": "backslash", ",": "comma", ".": "dot", "/": "slash"}
SHIFTED = {"!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7", "*": "8",
           "(": "9", ")": "0", "_": "minus", "+": "equal", "{": "bracket_left", "}": "bracket_right",
           ":": "semicolon", '"': "apostrophe", "~": "grave_accent", "|": "backslash",
           "<": "comma", ">": "dot", "?": "slash"}


def qcodes(ch):
    if ch.isalpha():
        return (["shift"] if ch.isupper() else []) + [ch.lower()]
    if ch.isdigit():
        return [ch]
    if ch in NAMED:
        return [NAMED[ch]]
    if ch in SHIFTED:
        return ["shift", SHIFTED[ch]]
    raise SystemExit(f"cannot type {ch!r}")


def main():
    path, mode, arg = sys.argv[1], sys.argv[2], sys.argv[3]
    s = socket.socket(socket.AF_UNIX)
    s.connect(path)
    f = s.makefile("rw")
    f.readline()

    def call(cmd, args=None):
        f.write(json.dumps({"execute": cmd, **({"arguments": args} if args else {})}) + "\n")
        f.flush()
        while True:
            reply = json.loads(f.readline())
            if "return" in reply or "error" in reply:
                return reply

    call("qmp_capabilities")
    combos = [arg.split("-")] if mode == "keys" else [qcodes(c) for c in arg + "\n"]
    for combo in combos:
        call("send-key", {"keys": [{"type": "qcode", "data": k} for k in combo]})
        time.sleep(0.03)


main()
