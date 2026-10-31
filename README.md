# OBSBot4Linux

Control OBSBOT webcams on Linux, currently the Tiny 2 Lite: every setting OBSBOT Center
offers, from the console and from the Dux (DankMaterialShell) bar widget
[obsbot4linux-dux](https://github.com/nofuturekid/obsbot4linux-dux).

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
  The live pose is sent on request: `{"id": 1, "args": ["live-pose", "on"]}` (`"off"` to stop)
  makes the daemon read the gimbal pose every second for as long as at least one connection has
  it on (a closed connection counts as off, a new one starts off; capability
  `live_pose_on_demand`). Without any, `pose` in the state events is refreshed only by the full
  read every 5 s. `live-pose` is rejected as a one-off CLI call.

A change made one way shows up the other way at once.

The camera is opened only for each command or poll and closed again. Controlling
the camera does not need exclusive access, so video calls and other programs keep
streaming while `obsbot4linux` reads and changes settings.

All control goes through ioctls on the camera's video node, as detection finds it
(see [Supported cameras](#supported-cameras)): the `/dev/v4l/by-id/...-video-index0` link,
else the first `/dev/videoN` with a matching V4L2 card name; with several supported cameras
connected, the first one found. The ioctls reach the vendor extension unit (framed commands,
queries, status block) and the UVC camera terminal (image controls). The gimbal moves with
OBSBOT Center's pose command `0x6444` (or its speed variant, see [Gimbal and zoom moves](#gimbal-and-zoom-moves)), not the UVC pan/tilt control, which ignores a target
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
- The same output carries `firmware` (the version string, e.g. `"6.2.8.1"`, query `0x0408`) and
  `model_info` (`{"product": 8, "platform": 8}`, query `0x1948`; product 8 is the Tiny 2 Lite).
  The daemon reads them once per connect; both are `null` when the camera does not answer.
  The serial number and UUID are not read. `obsbot4linux status` prints `firmware`.
- `status.anti_flicker` is `off`, `50hz`, `60hz` or `auto` (the camera detects the mains
  frequency itself); `status` also has `wake_up_zoom_percent` (0-100 like `digital_zoom_percent`,
  the zoom factor after waking is 1 + 3 * value / 100), `sleep_images` (indices 0-3 of the
  uploaded sleep images) and `default_work_mode` (`{"main", "sub"}`).

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

`box` is the tracked person's box as four normalised (0..1) coordinates: left, top, right,
bottom of the picture (checked by drawing it over the preview). Plain `obsbot4linux status` prints one line,
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

### Picture and sound

| Command | Does | Capability |
| --- | --- | --- |
| `led blink on\|off` | status LED blinks or burns steadily (`led 0-3` is still the brightness). The camera does not report it, so `status` has no field | `led_blink` |
| `exposure manual --time T --gain G` | manual exposure with the shutter time (100 us units, 1-2500; the camera uses the step at or below, e.g. 600 gives 500) and gain (1-64); both optional, both checked before anything is sent. Without them it only switches to manual as before | `exposure_values` |
| `audio volume 0-100` | microphone volume; `audio_volume` in the status reads it back, and `status.pickup_distance` follows (below 50 near, 50-84 standard, from 85 far) | `audio_volume` |
| `wake zoom 1.0-4.0` | zoom after waking up; read back as `status.wake_up_zoom_percent` (2.0 is 33) | `wake_zoom` |
| `wake mode <mode>` | tracking mode the camera starts in (the same names as `tracking`); read back as `status.default_work_mode`. The running mode does not change. `wake` alone still wakes the camera | `wake_mode` |
| `focus af continuous\|single\|manual` | the camera's own autofocus mode (`single` focuses once and holds; `manual` takes the position of `focus set`) | `focus_af_mode` |
| `focus point X Y` | focus once on a point, X and Y from 0.0 to 1.0 from the top left; the camera switches to `single` | `focus_point` |

`focus` in the status (query `0x3642`) reads `{"mode": "continuous", "position": 0, "point": [0.5, 0.5]}`:
the autofocus mode (`continuous`, `single`, `manual`), the manual position 0-100 (kept in the other
modes; `null` if the camera reports something else) and the centre of the focus window. `focus` and
`audio_volume` are `null` while the camera sleeps or did not answer the query. The focus commands
and their readback worked on a Tiny 2 Lite with and without a video stream running (the firmware
code mentions a "media busy" error without a pipeline; not seen). The camera does not acknowledge
these set commands, so a refused value only shows as an unchanged readback. A query the camera
leaves unanswered costs up to a second per full status read (every 5 s at most); this is not
backed off.

`image exposure_time` is refused unless the exposure is manual (a shutter time sent in automatic
exposure freezes it); use `exposure manual --time`.

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
    "return": { "pan": 0.0, "tilt": 0.0, "zoom": 1.0 },
    "views": [{ "id": 0, "name": "Bereich1", "pose": { "pan": 0.2, "tilt": 0.0, "zoom": 1.5 } }]
  },
  "state": {
    "zone_follow": true,
    "startup": { "pan": 0.0, "tilt": 0.0, "zoom": 1.0, "preset": null }
  }
}
```

`zone` is `null` when the camera does not answer. `state.zone_follow` and `state.startup` keep
their place and now come from the camera too (`null` without an answer); `startup.preset` is the
preset slot the start pose was taken from, `null` for an own pose. `startup.ai_mode` is the AI mode
of the initial state (see below), `null` when none is set.

- `zone return-here` stores the current pose (queries `0x0043` and `0x6804`) as the position the
  zone tracking returns to; `zone return-go` moves the camera there.
- `zone return` is always a pose, never `null`: the camera has no "not set" for it. The leading
  word of the `0x4ec4` answer is the preset id (0), and a fresh camera answers pan 0, tilt 0,
  zoom 1.0, so that cannot be told from a return position set to the same pose (#57).
- `startup show` prints the start pose (query `0x3884`) and the picture settings of the initial
  state (`0x3e84`); a setting the initial state does not set is `null`. `ai_mode` is the first byte
  of that block.
- `startup go` moves the camera to the start pose now (what `zone return-here` sent before 0.5.0).
  The camera ignores it while asleep or with the gimbal locked. It stops tracking when the initial
  state has no AI mode: the mode still reads as set, but the target stays `null` until you set a
  mode again (#79). With a mode in the initial state the camera returns to it on arrival and
  tracks again within about a second; it also powers on in that mode.
- `startup set --ai static|normal|upper-body|headless|group|none` sets that AI mode. `startup set`
  with a pose keeps the mode as it is unless `--ai` is given; `--ai` alone changes the mode and
  leaves the pose (and the other settings of the initial state) as read from the camera. `none`
  is the camera's "not set". Hand tracking, close-up and lower body are not offered: the values
  are not verified. `startup set` with a pose still clears the other picture settings of the
  initial state.

Zone views are named poses stored in the camera, the "areas" of OBSBOT Center's zone dialog. The
camera does not track by them (static analysis: the tracking code never reads them); they are
poses to go to, like presets with a name and no fixed number of slots. `zone.views` in the status
lists them (`null` when the camera did not answer; queries `0x1344`, then `0x1504` and `0x1444`
for each id). Capability: `zone_views`.

| Command | Does |
| --- | --- |
| `zone view list` | lists the views |
| `zone view save <name> [--pan P --tilt T --zoom Z]` | stores a view under the lowest free id (`0x1384`, then the name with `0x1544`); without a pose the current one is used (`0x0043` and `0x6804`). Name: 1-32 printable ASCII characters |
| `zone view rename <id> <name>` | renames a view (`0x1544`) |
| `zone view delete <id>` | deletes a view (`0x13c4`) |
| `zone view go <id>` | moves the camera to the view (`0x1604`). Not tested on the camera yet: whether the tracking mode stays, and whether it changes the next zone return, are open (see below) |

The daemon reads these seven values at start, after each command and then once a minute, not with
every poll. Capability: `zone_return`.

When the target leaves the area, the camera returns to the return position and, with `zone follow
on`, keeps tracking and takes the next person. A preset recall (`preset recall`, framed `0x39c4`) sets
a flag in the camera that turns the next such return into Static once; after that it keeps
tracking again. There is no other way to recall a preset. obsbot4linux avoids the flag
elsewhere: it reads the live pose from the gimbal controller (`0x0043`), not with the AI's pose
query `0x6604` (#67), and `startup go` uses the short command `00`, not the framed `0x3904` (#79).

### Hand tracking return position

The camera has its own return position for hand tracking, where the gimbal goes once the hand has
left the tracking range. It, the hand area and the axis settings are read from the camera (queries
`0x2504`, `0x2004` and `0x2484`) and appear next to `zone`, in the same angles (pan positive =
camera left, tilt positive = up):

```json
{
  "hand": {
    "area": {
      "pan_min": -45.0,
      "pan_max": 45.0,
      "tilt_min": -30.0,
      "tilt_max": 30.0
    },
    "return": { "pan": 20.1, "tilt": 4.9, "zoom": 1.0 },
    "pan_enabled": true,
    "tilt_enabled": true,
    "return_after": 2.0
  }
}
```

`return` is `null` while none is set, and `hand` is `null` when the camera does not answer.
`pan_enabled` and `tilt_enabled` are bytes 22 and 23 of the `0x2004` answer (1 / 1 in the
captures); `return_after` comes from `0x2484` and is `null` if the camera does not answer it (that
query was not captured). The camera's default area is 45 degrees to each side and 30 up and down.

- `hand return-here` stores the current pose (queries `0x0043` and `0x6804`) with `0x24c4`.
- `hand return-go` moves the camera to the stored position (`0x2584`). With none set, the camera
  goes to pan 0, tilt 0.
- `hand return-reset` clears it (`0x2544`); `return` is `null` again.
- `hand left` and `hand right` choose the tracked hand as before.
- `hand area --pan MIN MAX --tilt MIN MAX` sets the area the hand is tracked in (`0x2044`,
  `0x20c4`, `0x2144`, `0x21c4`, the same ranges as `zone area`); `hand area-reset` sets all four
  bounds back to the default (`0x25c4`, `0x2604`, `0x2644`, `0x2684`). Capability: `hand_area`.
- `hand pan on|off` and `hand tilt on|off` let the hand tracker move only one axis (`0x2344`,
  `0x23c4`). Capability: `hand_axes`.
- `hand return-after <seconds>` sets how long the hand may be outside the area before the camera
  goes to the return position (`0x2444`). Capability: `hand_return_after`.
- `hand gimbal on|off` lets the hand tracker move the gimbal or holds it (`0x26c4`; off also
  stops the gimbal; the camera does not store it, OBSBOT Center sends 0 while its area dialog is
  open and 1 on OK). Switch it back on. Capability: `hand_gimbal`.

The area, axis and return-time commands come from the firmware inventory and the Center capture of
the hand settings dialog; whether the hand AI mode (`16 03`) or a later mode change overwrites them
is not known yet.

The daemon reads the two values together with the zone values (see above). Capability:
`hand_return`.

### Gimbal and zoom moves

`gimbal --pan P --tilt T` is unchanged (command `0x6444`, joint angles, as fast as the motors go,
tracking switched to static first). New, all of them switch tracking to static first except `stop`:

| Command | Does | Capability |
| --- | --- | --- |
| `gimbal --pan P --tilt T --speed S` | the same move at S degrees per second on each axis (1-180, command `0x6544`). Arrives at the pose (pan 20 / tilt 10 at 5 deg/s took 4 s and read back 20.0 / 10.0) | `gimbal_speed` |
| `gimbal rate --pan R --tilt R` | keeps turning at R degrees per second (up to 178: from 179 the camera ignores the axis, positive = the way `--pan` / `--tilt` go) until `gimbal stop` or a rate of 0, like the joystick of OBSBOT Center (`0x6484`). One request starts it, **it turns until `gimbal stop` or a rate of 0**: the camera has no timeout, so a client that dies leaves the gimbal turning to its end (and tilting below -75 sends the camera to sleep). There is no daemon watchdog and no capability for it, so clients do not build a joystick on it; the timed rate `0x64c4` (speed for N ms) could be the safe form, see #80 The camera reverses the yaw of this command when exactly one of mirror and invert is on; obsbot4linux reads both and undoes it, so `--pan 5` always raises `pose.pan` (the firmware notes name more flips, `gimbal_flip`, `camera_flip` and a pitch invert; those are not handled and not tested) | none |
| `gimbal stop` | stops all axes (`0x6704`); does not change the AI mode | `gimbal_stop` |
| `gimbal center X Y` | experimental and hidden from `--help`, no capability: turns the gimbal so that the point X, Y (0.0-1.0 from the top left) of the picture moves to the centre (`0x0544`). **Only half works on a Tiny 2 Lite**, see below | none |
| `zoom F --speed S` | zooms smoothly to F (1.0-4.0) instead of jumping (`0x6884`); OBSBOT Center sends S = 30; 10 was clearly slower (about 2 s for 1.0 to 2.0) and 50 faster (about 1 s for 2.0 to 1.0) | `zoom_speed` |
| `zoom stop` | stops a zoom in progress (`0x68c4`) | `zoom_speed` |

`gimbal center` moved the gimbal in a Tiny 2 Lite only for points left of the centre (x below 0.5,
pan went negative, 0.3 gave -16 degrees, 0.0 gave -40 degrees, i.e. about 80 degrees across the
picture) and below it (y above 0.5, tilt went negative, 0.7 gave -10.8 degrees, about 54 degrees
across). For points right of the centre or above it the camera did not make the move (pan crept by
0.1-0.25 degrees per second in that direction, tilt up from the centre did nothing for 0.3 and
worked for 0.0); mirror and invert changed nothing. The cause is not known, so do not build a
click-to-centre on it yet. Not tried while AI tracking runs: the command switches to static first.

The pose that `0x0043` reports (bytes 6-11, what `status.pose` shows) is the joint angle that
`0x6444` sets, so moves and read-back match; the first six bytes are a second angle triple (by the
firmware notes the world frame) that read 1.4 / 11.5 / -45.8 degrees at the joint pose 0 / 0, and
are not read. The world-frame (Euler) variants `0x6404`, `0x6504`, `0x6584`
are therefore not exposed. The timed moves `0x65c4` / `0x6584` (arrive in N ms) have two 32-bit
words after the time that nobody knows; with 0 and 1 in every combination the camera did not move
(`0x6584` only with 0, 0), so they are not exposed either, nor is `0x64c4` (speed for N ms, same
words, not tried). `0x6484`, `0x6544`, `0x6704`, `0x6884`, `0x68c4` and
`0x0544` do not arm the zone Static flag (below).

### Tracking speed and framing

`speed standard|sport|headroom` is the vertical tracking type (`0x0cc4`, read back as
`status.tracking_speed`): `headroom` is the third value, a different vertical framing; only `sport`
fixes the tracking gain. `speed-mode lazy|slow|normal|fast|very-fast|auto` (`0x0944`) is the finer
speed of the tracking itself; it has no effect while `speed sport` is set or the part mode is not
`normal`. Gains are 0.25 / 0.5 / 0.75 / 1.0 / 1.25 for `lazy` to `very-fast`, `auto` adapts (static
analysis, the effect on the tracking was not measured). `framing` has the options the AI modes are
made of:

| Command | Does |
| --- | --- |
| `framing auto-zoom on\|off` | auto zoom of the human tracking (`0x0a44`) |
| `framing size long-shot\|medium-long-shot\|medium-shot\|medium-close-up\|close-up\|big-close-up` | auto zoom size (`0x0ac4`); the AI modes use `medium-shot` and `medium-close-up` |
| `framing zoom-speed slow\|medium\|fast` | how fast the auto zoom follows (`0x0b44`) |
| `framing offset --x X --y Y` | shifts the framing by a fraction of the frame, -0.16 to 0.16, either or both (`0x0bc4` / `0x0c44`; the camera clamps larger values, so they are refused). Which way positive goes was not checked |
| `framing part normal\|head-removal\|lower-body` | which part of the body is framed (`0x1584`) |

`status.framing` (queries `0x0984`, `0x0a84`, `0x0b04`, `0x0b84`, `0x0c04`, `0x0c84`, `0x15c4`) reads
them back; it is `null` when the camera does not answer:

```json
{
  "framing": {
    "speed_mode": "auto",
    "auto_zoom": false,
    "size": "long_shot",
    "zoom_speed": "slow",
    "offset": { "x": 0.0, "y": 0.0 },
    "part_mode": "normal"
  }
}
```

Every option is read back as set. A change of the AI mode overwrites some of them (read back:
`tracking upper-body` set auto zoom on and the size to `medium_shot`, `tracking headless` set auto
zoom off and the part mode to `head_removal`, and `tracking static` left both as they were), but
not `speed_mode`, the offset or the zoom speed. The daemon reads them with the zone values (after
each command, on an AI mode change and then once a minute). A camera that was never set reports
the zoom speed as 10; it reads back as `slow`, like 0. Capabilities: `speed_mode`,
`speed_headroom`, `framing`.

### Commands that arm the zone Static flag

A preset recall (`0x39c4`), `0x3904`, `0x3dc4` and `0x6604` make the next zone tracking return go
Static (see [Zone tracking and start pose](#zone-tracking-and-start-pose)). The flag is armed by
the command id alone, for the ranges `0x39c0`-`0x39ff`, `0x3dc0`-`0x3dff`, `0x6600`-`0x663f` and
`0x3900`-`0x393f` (static analysis of the camera's host process). None of the commands in this
section is in them (`0x0544`, `0x0944`, `0x0a44`, `0x0ac4`, `0x0b44`, `0x0bc4`, `0x0c44`, `0x1584`,
`0x6484`, `0x6544`, `0x6704`, `0x6884`, `0x68c4`, nor the read-back queries); a zone tracking return
after them was not run on the camera. The zone view commands (`0x1344`, `0x1384`, `0x13c4`, `0x1444`,
`0x1504`, `0x1544`, `0x1604`) and the hand commands (`0x2044`-`0x21c4`, `0x2344`-`0x2484`,
`0x25c4`-`0x2684`, `0x26c4`) are outside the ranges too, which a unit test checks; `zone view go`
is the one that matters, since it moves the gimbal like a preset recall does. Whether the camera
treats it like one inside is not known; a zone return after it is still to be run.

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
