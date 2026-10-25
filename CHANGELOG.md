# Changelog

All notable changes to OBSBot4Linux. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
