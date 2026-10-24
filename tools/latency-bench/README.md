# Latency bench (#191)

Measures the one-way latency from this PC to another one while the mouse
moves, using Mickey's own ping (round trip / 2).

1. Start the test mouse first: Mickey only reads the devices present when
   it starts. It needs write access to `/dev/uinput` (the `input` group).

   ```sh
   mkfifo /tmp/mouse && python3 bench-mouse.py < /tmp/mouse &
   exec 3>/tmp/mouse
   ```

2. Restart Mickey in bench mode: while the keyboard is on another PC it
   pings every 10 ms instead of every second.

   ```sh
   mickey quit; MICKEY_BENCH=1 mickey run &
   ```

3. Send the keyboard over, move for 30 s at 250 events a second (a real
   mouse sends 125 to 1000), come back:

   ```sh
   mickey go Desktop; sleep 1; echo "go 30 250" >&3; sleep 31; mickey home
   ```

   Mickey prints `one-way latency to Desktop over the last N pings: p50 …,
   p99 …`. Check with `mickey status` before `mickey home` that the focus
   is still there: the test mouse first moves right, and pushing past an
   edge that leads back brings the focus home early.

Wi-Fi adds its own jitter; the budget (p99 under 2 ms) is for a wired LAN.
