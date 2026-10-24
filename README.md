# OBSBot4Linux

Control OBSBOT webcams on Linux, currently the Tiny 2 Lite: every setting OBSBOT Center
offers, from the console and from the DMS bar widget
[dms-obsbot-control](https://github.com/nofuturekid/dms-obsbot-control).

The protocol comes from USB captures of OBSBOT Center, documented in
[obsbot-tiny2-lite-protocol](https://github.com/nofuturekid/obsbot-tiny2-lite-protocol);
commands are tested byte for byte against the captured frames.

## Design

One program, `obsbot4linux`:

- **`obsbot4linux daemon`** owns the camera state. It runs as a systemd user service,
  started on demand through the socket `$XDG_RUNTIME_DIR/obsbot4linux.sock`. After every
  change, from any client, it sends the new state to all connected clients, and
  it polls the camera in the background to notice changes the camera makes itself
  (a gesture switching AI tracking on, a factory reset, unplugging).
- **CLI** (`obsbot4linux status`, `obsbot4linux fov wide`, `obsbot4linux gimbal --pan 10`, ... with
  `--json`) talks to the daemon, or to the camera directly when no daemon runs.
- **`obsbot4linux watch`** keeps a connection open for the widget: state updates in,
  commands out, as JSON lines on stdout / stdin.

A change made one way shows up the other way at once.

The camera is opened only for each command or poll and closed again. Controlling
the camera does not need exclusive access, so video calls and other programs keep
streaming while `obsbot4linux` reads and changes settings.

All control goes through ioctls on `/dev/video0`: the vendor extension unit
(framed commands, queries, status block) and the UVC camera terminal (image
controls). The gimbal moves with OBSBOT Center's pose command `0x6444`, not the
UVC pan/tilt control, which ignores a target near the centre while the gimbal is
further away. What the camera does not report (the last pose sent, the exposure
mode) the daemon remembers.

## Status

Local only. The `obsbot4linux` CLI (library plus binary) controls the camera directly, with
`--json` for every command, and `obsbot4linux daemon` / `obsbot4linux watch` implement the
design above (JSON lines over `$XDG_RUNTIME_DIR/obsbot4linux.sock`, override with
`OBSBOT4LINUX_SOCKET`). The DMS widget uses `obsbot4linux watch` only. Other models would
each get a module behind the same commands once one is at hand to capture and test.
Builds on the Tiny 2 Lite support written for
[Tiny4Linux](https://github.com/OpenFoxes/Tiny4Linux) (EUPL-1.2).

## Install

Arch Linux, from this checkout:

```sh
cd packaging/arch && makepkg -si
systemctl --user enable --now obsbot4linux.socket
```

The socket starts `obsbot4linux.service` on the first connection. Without the
units, `obsbot4linux watch` starts a daemon itself.

## Licence

GPL-3.0-or-later, see [LICENSE](LICENSE). Code taken over from Tiny4Linux is
EUPL-1.2 and is distributed here under the GPL by the EUPL's compatibility clause
(Article 5); files with such code say so in their header.
