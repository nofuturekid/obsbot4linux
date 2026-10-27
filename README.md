# OBSBot4Linux

Control OBSBOT webcams on Linux, currently the Tiny 2 Lite: every setting OBSBOT Center
offers, from the console and from the DMS bar widget
[dms-obsbot-control](https://github.com/nofuturekid/dms-obsbot-control).

The protocol comes from USB captures of OBSBOT Center (the protocol notes are not public
at the moment); commands are tested byte for byte against the captured frames.

## Design

One program, `obsbot4linux`:

- **`obsbot4linux daemon`** owns the camera state. It runs as a systemd user service,
  started on demand through the socket `$XDG_RUNTIME_DIR/obsbot4linux.sock`. After every
  change, from any client, it sends the new state to all connected clients, and
  it polls the camera in the background to notice changes the camera makes itself
  (a gesture switching AI tracking on, a factory reset, unplugging).
- **CLI** (`obsbot4linux status`, `obsbot4linux fov wide`, `obsbot4linux gimbal --pan 10`, ... with
  `--json`) talks to the daemon, or to the camera directly when no daemon runs (or with
  `--direct`). Processes that talk to the camera take turns through a lock on its video node,
  waiting up to 5 s; video applications are not affected.
- **`obsbot4linux watch`** keeps a connection open for the widget: state updates in,
  commands out, as JSON lines on stdout / stdin.

A change made one way shows up the other way at once.

The camera is opened only for each command or poll and closed again. Controlling
the camera does not need exclusive access, so video calls and other programs keep
streaming while `obsbot4linux` reads and changes settings.

All control goes through ioctls on the camera's video node, as detection finds it
(see [Supported cameras](#supported-cameras)): the `/dev/v4l/by-id/...-video-index0` link,
else the first `/dev/videoN` with a matching V4L2 card name; with several supported cameras
connected, the first one found. The ioctls reach the vendor extension unit (framed commands,
queries, status block) and the UVC camera terminal (image controls). The gimbal moves with
OBSBOT Center's pose command `0x6444`, not the UVC pan/tilt control, which ignores a target
near the centre while the gimbal is further away. What the camera does not report (the last
pose sent, the exposure mode) the daemon remembers.

`gimbal` and the poses of presets, start and return positions take pan -150..150 and tilt
-70..90. Tilted below about -75 the Tiny 2 Lite goes to sleep (privacy) and parks at -85; woken
there, it returns to that pose and falls asleep again. Way out: `obsbot4linux wake`, then
`obsbot4linux gimbal --tilt 0` a few times within the next seconds.

## Status

The `obsbot4linux` CLI (library plus binary) controls the camera directly, with
`--json` for every command, and `obsbot4linux daemon` / `obsbot4linux watch` implement the
design above (JSON lines over `$XDG_RUNTIME_DIR/obsbot4linux.sock`, override with
`OBSBOT4LINUX_SOCKET`). The DMS widget uses `obsbot4linux watch` only. Other models would
each get a module behind the same commands once one is at hand to capture and test.
Builds on the Tiny 2 Lite support written for
[Tiny4Linux](https://github.com/OpenFoxes/Tiny4Linux) (EUPL-1.2).

## Supported cameras

| Model              | USB id      | Status                                   |
| ------------------ | ----------- | ---------------------------------------- |
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
{
  "model": "Tiny 2 Lite",
  "streaming": true,
  "streaming_apps": ["ffmpeg"],
  "...": "..."
}
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

### Tracking target

While AI tracking is on, `obsbot4linux --json status` and the state events of `obsbot4linux watch`
carry `target` (query `0x01c4`): whether the camera currently has someone to follow.

```json
{ "target": { "detected": true, "box": [0.174, 0.182, 0.677, 0.985] } }
```

- `detected: true`: the camera has a target (status LED blue).
- `detected: false`: tracking is on but the camera is searching (status LED yellow); `box` is the
  last one it had.
- `null`: tracking is Static (LED green), no camera is connected, or the reply was unusable.

`box` is four normalised (0..1) coordinates in the order the camera sends them; which corner or
size each one is has not been established. Plain `obsbot4linux status` prints one line,
`target: detected` or `target: searching`, while tracking. The daemon asks once a second, only
while tracking is on, and sends a state event when `detected` changes. Box movement alone sends
none; the latest box is included whenever an event goes out for another reason. Capability: `target`.

### Limits

`limits` in the status says what `gimbal`, the preset, start and return poses and `zone area`
accept, as [min, max] (pan positive = camera left, tilt positive = up):

```json
{
  "limits": {
    "pan": [-150.0, 150.0],
    "tilt": [-70.0, 90.0],
    "zoom": [1.0, 4.0]
  }
}
```

### Zone tracking and start pose

`obsbot4linux --json status` and the state events of `obsbot4linux watch` carry the zone
settings, read from the camera (queries `0x4c44` / `0x4cc4` / `0x4d44` / `0x4dc4`, `0x4e44`,
`0x4ec4`, `0x3884`) and not from what was sent last. Angles are the ones `zone area` takes: pan
positive = camera left, tilt positive = up.

```json
{
  "zone": {
    "pan_min": -10.0,
    "pan_max": 10.0,
    "tilt_min": -5.0,
    "tilt_max": 5.0,
    "follow": true,
    "return": { "pan": 0.0, "tilt": 0.0, "zoom": 1.0 }
  },
  "state": {
    "zone_follow": true,
    "startup": { "pan": 0.0, "tilt": 0.0, "zoom": 1.0, "preset": null }
  }
}
```

`zone` is `null` when the camera does not answer. `state.zone_follow` and `state.startup` keep
their place and now come from the camera too (`null` without an answer); `startup.preset` is the
preset slot the start pose was taken from, `null` for an own pose.

- `zone return-here` stores the current pose (queries `0x6604` and `0x6804`) as the position the
  zone tracking returns to; `zone return-go` moves the camera there.
- `startup go` moves the camera to the start pose now (what `zone return-here` sent before 0.5.0).

The daemon reads these seven values at start, after each command and then once a minute, not with
every poll. Capability: `zone_return`.

### Hand tracking return position

The camera has its own return position for hand tracking, where the gimbal goes once the hand has
left the tracking range. It and the hand area are read from the camera (queries `0x2504` and
`0x2004`) and appear next to `zone`, in the same angles (pan positive = camera left, tilt
positive = up):

```json
{
  "hand": {
    "area": {
      "pan_min": -45.0,
      "pan_max": 45.0,
      "tilt_min": -30.0,
      "tilt_max": 30.0
    },
    "return": { "pan": 20.1, "tilt": 4.9, "zoom": 1.0 }
  }
}
```

`return` is `null` while none is set, and `hand` is `null` when the camera does not answer. The
area is only read; obsbot4linux has no command to change it (the camera's default is 45 degrees to
each side and 30 up and down).

- `hand return-here` stores the current pose (queries `0x6604` and `0x6804`) with `0x24c4`.
- `hand return-go` moves the camera to the stored position (`0x2584`). With none set, the camera
  goes to pan 0, tilt 0.
- `hand return-reset` clears it (`0x2544`); `return` is `null` again.
- `hand left` and `hand right` choose the tracked hand as before.

The daemon reads the two values together with the zone values (see above). Capability:
`hand_return`.

Adding a model: an entry in `MODELS` in `src/models.rs` (name, USB id, by-id and card name
patterns) makes it detected. Its commands and status decoding differ per camera and today exist
for the Tiny 2 Lite only; the plan is one module per model behind the same commands, once a
camera is at hand to capture and test (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Install

Download a package from the
[releases](https://github.com/nofuturekid/obsbot4linux/releases) (x86_64 and aarch64;
`SHA256SUMS` lists the checksums), then install it, with `VER` set to the release's version
(for example `VER=0.6.0`):

```sh
sudo apt install ./obsbot4linux_${VER}_amd64.deb             # Debian, Ubuntu (arm64 too)
sudo dnf install ./obsbot4linux-$VER-1.x86_64.rpm            # Fedora, openSUSE: zypper install
sudo pacman -U obsbot4linux-$VER-1-x86_64.pkg.tar.zst        # Arch Linux, Arch Linux ARM (aarch64)
tar -xzf obsbot4linux-$VER-x86_64-linux.tar.gz               # tarball: binary + systemd/ units
```

The packages install `/usr/bin/obsbot4linux` and the user units, and do not enable
anything. Enable the socket:

```sh
systemctl --user enable --now obsbot4linux.socket
```

The socket starts `obsbot4linux.service` on the first connection. Without the
units, `obsbot4linux watch` starts a daemon itself. From the tarball, put the
binary on your `PATH` and copy `systemd/*` to `~/.config/systemd/user/`. The service runs
`/usr/bin/obsbot4linux`; for a binary elsewhere, `systemctl --user edit obsbot4linux.service`
and point it there (the empty `ExecStart=` clears the packaged one):

```ini
[Service]
ExecStart=
ExecStart=%h/.local/bin/obsbot4linux daemon
```

Build from source (Rust 1.88 or newer):

```sh
cargo build --release --locked      # target/release/obsbot4linux
cd packaging/arch && makepkg -si    # Arch package of the committed HEAD
```

## Licence

GPL-3.0-or-later, see [LICENSE](LICENSE). Code taken over from Tiny4Linux is
EUPL-1.2 and is distributed here under the GPL by the EUPL's compatibility clause
(Article 5); files with such code say so in their header.
