# Changelog

All notable changes to OBSBot4Linux. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `firmware` (e.g. `6.2.8.1`, query `0x0408`) and `model_info` (`product`, `platform`, query
  `0x1948`) in `status` and the state events, read once per connect; `obsbot4linux status`
  prints the firmware. The serial number and UUID are not read (#63).
- `startup show` prints the start pose (`0x3884`) and the picture settings of the initial state
  (`0x3e84`) (#89).
- `status` shows `wake_up_zoom_percent`, `sleep_images` and `default_work_mode` (#82).

### Changed

- `status.anti_flicker` is a name now (`off`, `50hz`, `60hz`, `auto`) instead of the V4L2
  number; the camera can report automatic detection, which was passed on as a bare 3 (#82).

### Fixed

- `startup go` no longer turns the next zone tracking return into Static. It sends the short
  command `00` instead of the framed `0x3904`, which arms that flag in the camera (#79). The
  camera ignores the short command while asleep or with the gimbal locked.
- The error code of a rejected query ("camera rejected query ...") was read from the wrong place.
  Those answers have a 16-byte header with the code (a signed 16-bit number, e.g. -3) in an
  extension; it is now read from there and shown in decimal (#62).

## [0.8.0] - 2026-10-31

### Changed

- The daemon reads the live gimbal pose (`0x0043`) every second only while a client wants it.
  A `watch` connection asks with the request `["live-pose", "on"]` (and `"off"`); the poll runs
  while at least one connection has it on, a closed connection counts as off, and a new one
  starts off. Without a subscriber `pose` in the state events changes only with the full read
  every 5 s, so fewer queries hold the camera lock. New capability `live_pose_on_demand`; clients
  that never ask no longer get a pose every second (#68).

## [0.7.3] - 2026-10-31

### Fixed

- With zone tracking and "follow new targets" on, the camera switched to Static each time the
  target left the area and the gimbal returned, instead of taking the next person. obsbot4linux
  caused it: the live pose came from the AI query `0x6604`, polled every second, and every
  `0x6604` arms a flag in the camera that turns the next return into Static. The pose now comes
  from the gimbal controller (`0x0043`), which does not set the flag and follows AI tracking just
  the same (#67).

## [0.7.2] - 2026-10-27

### Fixed

- The daemon and a `--direct` call, or two direct calls, could talk to the camera at the same
  time. The vendor unit has one answer buffer and answers are matched by sequence number only,
  so one process could take the other's answer or time out. Every camera handle now holds an
  exclusive lock on the video node; the others wait up to 5 s ("the camera is busy" after that).
  Video applications do not take the lock and are not affected (#45).

## [0.7.1] - 2026-10-27

### Added

- `limits` in the status and the daemon's state events: the pan, tilt and zoom ranges that
  `gimbal`, the poses and `zone area` accept, so clients need not hardcode them (#58).

### Changed

- `zone area` takes the gimbal range, pan -150..150 and tilt -70..90, instead of OBSBOT Center's
  -100..100 and -30..30. Measured on a Tiny 2 Lite: zone tracking keeps to a tilt of 45 (stopped
  at 48.7; 90.7 without zone). Pan beyond 100 is only confirmed by the camera's read-back (#58).

### Fixed

- Tilt below -75 puts the Tiny 2 Lite to sleep (privacy), and woken there it returns to that pose
  and sleeps again. `gimbal` and the poses now stop at tilt -70, OBSBOT's controllable limit,
  instead of -89. The "-90 reads back as +90" noted in 0.7.0 was this sleep. The README shows the
  way out of the loop (#58).

## [0.7.0] - 2026-10-27

### Added

- A daemon that `watch` starts itself appends its errors to `daemon.log` next to `state.json`;
  "cannot reach the daemon" names that file (#42).
- Release artifacts carry a build provenance attestation (#36).

### Changed

- `gimbal`, `preset save`, `startup set` and `zone`/`hand return-here` reject poses outside
  pan -150..150 and tilt -89..90 (the measured mechanical range of the Tiny 2 Lite; -90 reads back
  as +90) and zoom 1.0-4.0, instead of sending them (#32, #39).
- `watch` exits with 1 when the daemon closes the connection, and with 0 when stdin or stdout
  close (#42).
- The daemon exits non-zero when its camera worker stops, so systemd's `Restart=on-failure`
  applies (#42).
- The daemon opens the camera once per tick instead of three to four times (#46).
- Rust 1.88 is the minimum version (`rust-version`), and CI tests with it. CI and release builds
  use a pinned toolchain (#33, #36).
- Releases are only built from commits on `main`, and run fmt and clippy before the build (#35).
- `packaging/arch/PKGBUILD` carries the release version; a release fails when it differs from
  the tag (#43).

### Fixed

- A `watch` client that stalled during a broadcast stayed connected but got no more events; its
  connection is now closed, so it reconnects (#30).
- Two daemons started at the same time could both bind the socket and poll the camera. A lock
  next to the socket now keeps it to one; the socket is created with mode 0600 from the start
  (#31).
- `gimbal --tilt 1000` or `--pan -1000` became "keep this axis" on the wire and still reported
  success (#32).
- `auto-sleep off` with a stored time of 0 s left auto-sleep on; it now sends off with 600 s
  (measured) (#37).
- A selector of the wrong length failed with a raw `ENOBUFS` instead of a clear error (#38).
- `--json status` and `preset list` no longer make the daemon re-read everything and broadcast
  (#40).
- `--direct` writes to `state.json` are no longer overwritten by the daemon's cache, and the file
  is written with a unique, synced temp file (#41).
- With two cameras attached, the same one is picked every time (#45).
- The rpm installs `LICENSE` also with `--excludedocs` (#43).
- README: Rust version, install examples for the current release, a systemd drop-in for a binary
  outside `/usr/bin`, and the device node that is actually used (#33, #34, #44).

## [0.6.0] - 2026-10-25

### Added

- `hand` in `obsbot4linux --json status` and the daemon's state events: the hand tracking area
  (`pan_min`, `pan_max`, `tilt_min`, `tilt_max`) and the `return` position (`pan`, `tilt`, `zoom`,
  `null` while none is set), read from the camera (queries `0x2004`, `0x2504`). `null` when the
  camera does not answer.
- `hand return-here` sets the current pose as the hand tracking return position (`0x24c4`),
  `hand return-go` moves the camera there (`0x2584`) and `hand return-reset` clears it (`0x2544`);
  new capability `hand_return`. `hand left` and `hand right` work as before.

## [0.5.0] - 2026-10-25

### Added

- `zone` in `obsbot4linux --json status` and the daemon's state events: the zone area
  (`pan_min`, `pan_max`, `tilt_min`, `tilt_max`), `follow` and the `return` position
  (`pan`, `tilt`, `zoom`), read from the camera (queries `0x4c44`, `0x4cc4`, `0x4d44`,
  `0x4dc4`, `0x4e44`, `0x4ec4`). `null` when the camera does not answer.
- `zone return-go` moves the camera to the zone return position (`0x4f44`); new capability
  `zone_return`.
- `startup go` moves the camera to the start pose (`0x3904`).

### Changed

- `state.zone_follow` and `state.startup` in the status are read from the camera (`0x4e44`,
  `0x3884`) instead of the state file; the keys are unchanged. They are `null` when the camera
  does not answer, and the state file no longer stores them. The daemon asks for the zone values
  at start, after each command and once a minute.

### Fixed

- `zone return-here` sent `0x3904`, which moves the gimbal to the boot pose and stores nothing
  (#16). It now reads the current pose (`0x6604`, `0x6804`) and sets it as the zone return
  position with `0x4e84`. The old behaviour is `startup go`.

## [0.4.0] - 2026-10-25

### Added

- `target` in `obsbot4linux --json status` and in the daemon's state events: the AI tracking
  target from query `0x01c4`, `{"detected": bool, "box": [..4 floats..]}`, or `null` while
  tracking is Static, no camera is connected or the reply is unusable. `detected` is true
  while the camera has a target (status LED blue) and false while it searches (yellow); `box`
  is then the last one. The daemon asks once a second, only while tracking is on, and sends an
  event when `detected` changes, not for box movement alone. Plain `status` prints
  `target: detected` or `target: searching` while tracking. New capability `target`.

## [0.3.1] - 2026-10-25

### Fixed

- `streaming` missed the most common streamers: the DMS widget's Qt Multimedia preview holds
  the node open without mapping it, and Chrome captures in a sandboxed process whose `/proc`
  entries are unreadable. `streaming` now asks the kernel (`VIDIOC_REQBUFS` with count 0
  fails with `EBUSY` while another handle owns the capture buffers), which sees every
  application. `streaming_apps` stays the best-effort `/proc` list and can be empty while
  `streaming` is true.

## [0.3.0] - 2026-10-25

### Added

- `streaming` (bool) and `streaming_apps` (process names, sorted, deduplicated) in
  `obsbot4linux --json status` and in the daemon's state events: whether an application
  streams from the camera. Detected from `/proc` (video node open and its buffers mapped);
  `false` and `[]` while no camera is connected. The daemon checks every second and sends an
  event when it changes. Only mmap-based capture and processes of the same user are seen.

## [0.2.0] - 2026-10-25

### Added

- `model` and `device` keys in `obsbot4linux --json status` and in the daemon's state
  events (`obsbot4linux watch`): the camera's human name and the V4L2 node to stream from
  (the stable `/dev/v4l/by-id/...-video-index0` link when available). Both are `null`
  while no camera is connected; all existing keys are unchanged.
- `obsbot4linux detect`: exit 0 and the model (`--json`: `{model, device}`) when a supported
  camera is connected, exit 1 without output when not. Enumeration only, no daemon and no
  camera command.
- "Supported cameras" section in the README.

## [0.1.0] - 2026-10-25

### Added

- `obsbot4linux` for the OBSBOT Tiny 2 Lite: every command with `--json`;
  commands tested byte for byte against OBSBOT Center captures.
- `obsbot4linux daemon` owns the camera state and broadcasts every change over
  `$XDG_RUNTIME_DIR/obsbot4linux.sock`; the CLI is a client of it and falls back
  to direct access. The camera is opened per operation, so video calls keep
  streaming.
- `obsbot4linux watch`: JSON lines for the DMS widget, state events out,
  requests in.
- Settings: sleep / wake, AI tracking mode and speed, HDR, exposure (global,
  face, manual over UVC), zoom, field of view, gimbal (Center's pose command),
  status LED, mirror, invert, auto-sleep, sleep background, audio, gestures,
  hand side, zone tracking with area and return position, image controls,
  autofocus, camera presets, start pose, factory reset.
- Live gimbal pose (query `0x6604`) in the status, read every second by the
  daemon.
- Socket-activated systemd user units (`obsbot4linux.socket` / `.service`).
- Release packaging: GitHub Actions run fmt, clippy and tests on every push; a
  `vX.Y.Z` tag builds tarballs, `.deb`, `.rpm` and Arch `.pkg.tar.zst` packages
  (x86_64 and aarch64) with `SHA256SUMS` and creates the GitHub release. The
  PKGBUILD builds either the local checkout or a release tarball.

### Changed

- Renamed from Tiny2Lite4Linux / `t2l` to OBSBot4Linux / `obsbot4linux`, so further OBSBOT
  models can join as modules.

### Fixed

- Selecting global or face exposure no longer freezes the automatic exposure
  (a manual-exposure frame was sent along).
