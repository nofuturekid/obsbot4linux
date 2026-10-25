# Changelog

All notable changes to OBSBot4Linux. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
