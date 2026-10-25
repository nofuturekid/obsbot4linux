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

The `obsbot4linux` CLI (library plus binary) controls the camera directly, with
`--json` for every command, and `obsbot4linux daemon` / `obsbot4linux watch` implement the
design above (JSON lines over `$XDG_RUNTIME_DIR/obsbot4linux.sock`, override with
`OBSBOT4LINUX_SOCKET`). The DMS widget uses `obsbot4linux watch` only. Other models would
each get a module behind the same commands once one is at hand to capture and test.
Builds on the Tiny 2 Lite support written for
[Tiny4Linux](https://github.com/OpenFoxes/Tiny4Linux) (EUPL-1.2).

## Supported cameras

| Model | USB id | Status |
| --- | --- | --- |
| OBSBOT Tiny 2 Lite | `3564:fef9` | Full support, tested on firmware 6.2.8.1 |

Detection only enumerates (`/dev/v4l/by-id`, else the V4L2 card name) and sends nothing to the
camera. Two ways to use it:

- `obsbot4linux detect` exits 0 and prints the model when a supported camera is connected,
  exits 1 with no output when not. It does not start or ask the daemon, so it is cheap enough
  to call every few seconds. `--json` prints `{"model":"Tiny 2 Lite","device":"/dev/v4l/by-id/...-video-index0"}`.
- `obsbot4linux --json status` and the state events of `obsbot4linux watch` carry two top-level
  keys: `model` (the human name) and `device` (the V4L2 node to stream from, the stable
  `/dev/v4l/by-id/...-video-index0` link when there is one, else `/dev/videoN`). Both are
  `null` while no camera is connected.

### Streaming applications

`obsbot4linux --json status` and the state events of `obsbot4linux watch` also carry
`streaming` (bool) and `streaming_apps` (the process names, deduplicated and sorted, `[]` when
none). Without a camera they are `false` and `[]`. The daemon checks every second and sends an
event as soon as they change; no camera command is involved.

```json
{"model": "Tiny 2 Lite", "streaming": true, "streaming_apps": ["ffmpeg"], "...": "..."}
```

`streaming` is the reliable one and comes from the kernel. uvcvideo lets only one handle own
the capture buffers, so obsbot4linux opens the video node and requests zero buffers
(`VIDIOC_REQBUFS`, count 0); `EBUSY` means some application streams. This sees every
application, also sandboxed ones like Chrome and ones that do not map the node like Qt
Multimedia. While nobody streams, the probe owns the buffer queue for a moment and releases it
at once; it does not disturb a running stream. Any other error counts as not streaming.

`streaming_apps` is best effort, only for names: a process that streams has the camera's
`/dev/videoN` open and has mapped its buffers (the node appears in `/proc/<pid>/maps`). A
process that only holds the node open, like a control panel, is not listed. Limits: only
mmap-based capture is seen, processes of other users and sandboxed processes are not readable,
and obsbot4linux's own process is left out. So the list can be empty while `streaming` is
`true`; trust `streaming` for "is the camera in use".

Adding a model: an entry in `MODELS` in `src/models.rs` (name, USB id, by-id and card name
patterns) makes it detected. Its commands and status decoding differ per camera and today exist
for the Tiny 2 Lite only; the plan is one module per model behind the same commands, once a
camera is at hand to capture and test (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Install

Download a package from the
[releases](https://github.com/nofuturekid/obsbot4linux/releases) (x86_64 and aarch64;
`SHA256SUMS` lists the checksums), then install it:

```sh
sudo apt install ./obsbot4linux_0.1.0_amd64.deb              # Debian, Ubuntu (arm64 too)
sudo dnf install ./obsbot4linux-0.1.0-1.x86_64.rpm           # Fedora, openSUSE: zypper install
sudo pacman -U obsbot4linux-0.1.0-1-x86_64.pkg.tar.zst       # Arch Linux, Arch Linux ARM (aarch64)
tar -xzf obsbot4linux-0.1.0-x86_64-linux.tar.gz              # tarball: binary + systemd/ units
```

The packages install `/usr/bin/obsbot4linux` and the user units, and do not enable
anything. Enable the socket:

```sh
systemctl --user enable --now obsbot4linux.socket
```

The socket starts `obsbot4linux.service` on the first connection. Without the
units, `obsbot4linux watch` starts a daemon itself. From the tarball, put the
binary on your `PATH` and copy `systemd/*` to `~/.config/systemd/user/`.

Build from source (Rust 1.85 or newer):

```sh
cargo build --release --locked      # target/release/obsbot4linux
cd packaging/arch && makepkg -si    # Arch package of this checkout
```

## Licence

GPL-3.0-or-later, see [LICENSE](LICENSE). Code taken over from Tiny4Linux is
EUPL-1.2 and is distributed here under the GPL by the EUPL's compatibility clause
(Article 5); files with such code say so in their header.
