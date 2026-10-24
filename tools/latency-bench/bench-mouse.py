#!/usr/bin/env python3
"""A test mouse through uinput for the latency bench (#191). Prints 'ready',
then on each 'go <seconds> [<events per second>]' line (500 by default)
moves 500 units right, away from a neighbour on the left, then shakes in
place for that long: 4 units right, then 4 left, every 10 events."""A test mouse through uinput for the latency bench (#191). Prints 'ready',
then on each 'go <seconds>' line moves for that long at 500 events/s,
4 units right then 4 left every 10 events, so the cursor stays put."""
import fcntl, os, struct, sys, time
UI_SET_EVBIT, UI_SET_KEYBIT, UI_SET_RELBIT = 0x40045564, 0x40045565, 0x40045566
UI_DEV_SETUP, UI_DEV_CREATE, UI_DEV_DESTROY = 0x405C5503, 0x5501, 0x5502
EV_SYN, EV_KEY, EV_REL, REL_X, REL_Y, BTN_LEFT = 0, 1, 2, 0, 1, 0x110
fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
for ev in (EV_KEY, EV_REL):
    fcntl.ioctl(fd, UI_SET_EVBIT, ev)
fcntl.ioctl(fd, UI_SET_KEYBIT, BTN_LEFT)
for rel in (REL_X, REL_Y):
    fcntl.ioctl(fd, UI_SET_RELBIT, rel)
fcntl.ioctl(fd, UI_DEV_SETUP, struct.pack("HHHH80sI", 3, 0x1234, 0x5678, 1, b"Mickey Bench Mouse", 0))
fcntl.ioctl(fd, UI_DEV_CREATE)
print("ready", flush=True)
for line in sys.stdin:
    words = line.split()
    seconds = float(words[1]) if len(words) > 1 else 10
    rate = float(words[2]) if len(words) > 2 else 500
    # Away from the left edge first: pushing past an edge that leads back
    # home would end the bench early.
    for _ in range(50):
        os.write(fd, struct.pack("llHHi", 0, 0, EV_REL, REL_X, 10) + struct.pack("llHHi", 0, 0, EV_REL, REL_Y, 0) + struct.pack("llHHi", 0, 0, EV_SYN, 0, 0))
        time.sleep(0.004)
    end, n = time.monotonic() + seconds, 0
    while time.monotonic() < end:
        dx = 4 if (n // 10) % 2 == 0 else -4
        os.write(fd, struct.pack("llHHi", 0, 0, EV_REL, REL_X, dx) + struct.pack("llHHi", 0, 0, EV_SYN, 0, 0))
        n += 1
        time.sleep(1 / rate)
    print(f"moved {n} times", flush=True)
fcntl.ioctl(fd, UI_DEV_DESTROY)
