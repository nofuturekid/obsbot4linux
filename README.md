# Tiny2Lite4Linux

Control the OBSBOT Tiny 2 Lite webcam on Linux: every setting OBSBOT Center
offers, from the console and from the DMS bar widget
[dms-obsbot-control](https://github.com/nofuturekid/dms-obsbot-control).

The protocol comes from USB captures of OBSBOT Center, documented in
[obsbot-tiny2-lite-protocol](https://github.com/nofuturekid/obsbot-tiny2-lite-protocol);
commands are tested byte for byte against the captured frames.

## Design

One program, `t2l`:

- **`t2l daemon`** owns the camera state. It runs as a systemd user service,
  started on demand through the socket `$XDG_RUNTIME_DIR/t2l.sock`. After every
  change, from any client, it sends the new state to all connected clients, and
  it polls the camera in the background to notice changes the camera makes itself
  (a gesture switching AI tracking on, a factory reset, unplugging).
- **CLI** (`t2l status`, `t2l fov wide`, `t2l gimbal --pan 10`, ... with
  `--json`) talks to the daemon, or to the camera directly when no daemon runs.
- **`t2l watch`** keeps a connection open for the widget: state updates in,
  commands out, as JSON lines on stdout / stdin.

A change made one way shows up the other way at once.

The camera is opened only for each command or poll and closed again. Controlling
the camera does not need exclusive access, so video calls and other programs keep
streaming while `t2l` reads and changes settings.

All control goes through ioctls on `/dev/video0`: the vendor extension unit
(framed commands, queries, status block) and the UVC camera terminal (image
controls). The gimbal moves with OBSBOT Center's pose command `0x6444`, not the
UVC pan/tilt control, which ignores a target near the centre while the gimbal is
further away. What the camera does not report (the last pose sent, the exposure
mode) the daemon remembers.

## Status

Local only, in planning. Builds on the Tiny 2 Lite support written for
[Tiny4Linux](https://github.com/OpenFoxes/Tiny4Linux) (EUPL-1.2); the licence of
this project is still to be decided.
