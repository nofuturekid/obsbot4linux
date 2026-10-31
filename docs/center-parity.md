# OBSBOT Center parity (Tiny 2 Lite)

An inventory of what OBSBOT Center for Windows offers for the Tiny 2 Lite against what
obsbot4linux does today. It is for discussion, not a plan: nothing here is scheduled.

## Sources and how to read the table

- **SA**: static analysis of Center 2.0.15.186's device library (`libdev.dll`). Derived from code,
  nothing was sent to a camera. A command id found there is reliable as an id; whether the Tiny 2
  Lite firmware accepts it is a separate question.
- **CM**: the command notes written from USB captures of Center talking to a Tiny 2 Lite
  (firmware 6.2.7.4 and 6.2.8.1). They mark a command _verified_ (replayed from Linux, effect or
  status change observed) or _captured_ (seen in the captures with a clear match to a click, not
  replayed).
- **CN**: the capture log, which records which Center settings were clicked per capture. It is the
  evidence for "in Center UI".
- **OB**: obsbot4linux 0.6.0: README, CHANGELOG, `obsbot4linux --help` and the subcommand help,
  `CAPABILITIES` in `src/camera.rs`.

Columns:

- **In Center UI**: `yes` = a click was captured (CN); `code only` = the device library has the
  method but no capture shows the UI using it (or Center does not import it); `unknown` = Center
  imports the method but nothing says whether, or for which models, the UI shows it.
- **obsbot4linux**: the CLI command, `partial` where only a part is covered, or `—`.
- **Capability**: the name in the `capabilities` list of `status --json`.
- **Verified on camera**: `yes` = CM says verified; `captured` = CM says captured only; `no` = no
  evidence on a Tiny 2 Lite. This is about the command id, not about obsbot4linux's code, which is
  tested byte for byte against the captured frames.

Only feature names and command ids are given. Ids marked "(SA)" come from the static analysis and
were never sent to a camera.

## Tracking / AI

| Feature (as Center shows it)                                                                        | Command id(s)                                                   | In Center UI                                                      | obsbot4linux                                                  | Capability    | Verified on camera                               |
| --------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- | ----------------------------------------------------------------- | ------------------------------------------------------------- | ------------- | ------------------------------------------------ |
| AI tracking mode: Normal, Upper body, Close-up, Headless, Lower body                                | short `16 02 m n`                                               | yes (CN)                                                          | `tracking normal\|upper-body\|close-up\|headless\|lower-body` | `tracking`    | captured (CM); status bytes `0x18`/`0x1c` follow |
| Hand tracking mode                                                                                  | short `16 02` (`03`)                                            | yes (CN)                                                          | `tracking hand`                                               | `tracking`    | captured (CM)                                    |
| Group tracking mode                                                                                 | short `16 02` (`01`)                                            | yes (CN)                                                          | `tracking group`                                              | `tracking`    | captured (CM)                                    |
| Tracking off (Static)                                                                               | short `16 02` (`00`)                                            | yes (CN)                                                          | `tracking static`                                             | `tracking`    | captured (CM)                                    |
| Tracking speed: Standard / Sport                                                                    | `0x0cc4`                                                        | yes (CN)                                                          | `speed standard\|sport`                                       | `speed`       | yes (CM; `0x0944` from SA has no effect)         |
| Hand tracking: left / right hand                                                                    | `0x2244`                                                        | yes (CN)                                                          | `hand left\|right`                                            | `hand`        | yes (CM, in Center)                              |
| Hand tracking area (pan/tilt limits) and custom-area switch                                         | `0x2044`, `0x20c4`, `0x2144`, `0x21c4`, `0x26c4`; read `0x2004` | yes (CN, hand settings dialog)                                    | partial: read only (`hand.area` in status)                    | `hand`        | set: captured; read: yes (CM)                    |
| Hand tracking area reset                                                                            | `0x25c4`, `0x2604`, `0x2644`, `0x2684` (SA)                     | code only                                                         | —                                                             | —             | no                                               |
| Hand tracking return position (set / go / clear)                                                    | `0x24c4` / `0x2584` / `0x2544`; read `0x2504`                   | yes for set (CN); go and clear are code only (not sent by Center) | `hand return-here\|return-go\|return-reset`                   | `hand_return` | yes (CM)                                         |
| Zone tracking on / off                                                                              | `0x08c4` + `0x5084` (pair, ~100 ms apart)                       | yes (CN)                                                          | `zone on\|off`                                                | `zone`        | yes (CM)                                         |
| Zone tracking area (pan/tilt limits)                                                                | `0x4c04`, `0x4c84`, `0x4d04`, `0x4d84`; read `0x4c44`..`0x4dc4` | yes (CN)                                                          | `zone area`                                                   | `zone`        | yes (CM)                                         |
| Zone: automatically follow new targets                                                              | `0x4e04`; read `0x4e44`                                         | yes (CN)                                                          | `zone follow`                                                 | `zone_follow` | yes (CM)                                         |
| Zone: return to start position                                                                      | set `0x4e84`, go `0x4f44`, read `0x4ec4`                        | yes for set (CN); go is code only                                 | `zone return-here\|return-go`                                 | `zone_return` | yes (CM)                                         |
| Zone area reset                                                                                     | `0x4f04`, `0x4f84`, `0x4fc4`, `0x5004`, `0x5044` (SA)           | code only                                                         | —                                                             | —             | no                                               |
| Pick or clear a tracking target (double-click in preview: by position, box, biggest, central, none) | `0x0404`, `0x0444`, `0x0484`, `0x04c4`, `0x0504`, `0x0684` (SA) | code only (preview interaction not captured)                      | —                                                             | —             | no                                               |
| Enable one tracking mode / AI work mode as framed command                                           | `0x0584`, `0x0284` (SA)                                         | code only (Center uses the short command)                         | —                                                             | —             | no                                               |
| AI auto zoom                                                                                        | `0x0a44` (SA)                                                   | code only (not imported by Center)                                | —                                                             | —             | no                                               |
| Target zoom type, target view type                                                                  | `0x06c4`, `0x0704` (SA)                                         | unknown                                                           | —                                                             | —             | no                                               |
| Composition lines: horizontal / vertical offset                                                     | `0x0bc4`, `0x0c04`, `0x0c44`, `0x0c84` (SA)                     | unknown                                                           | —                                                             | —             | no                                               |
| Auto horizontal framing by face orientation                                                         | `0x5204`, `0x5244` (SA)                                         | unknown                                                           | —                                                             | —             | no                                               |
| Virtual track (and its gesture/enable variants)                                                     | `0xac02`..`0xaec2` (SA)                                         | unknown                                                           | —                                                             | —             | no                                               |
| Auto framing mode                                                                                   | short `0d 02 a b` (SA)                                          | unknown (likely Meet family)                                      | —                                                             | —             | no                                               |
| Generic gimbal / control parameters                                                                 | `0x3f44`, `0x3f84`, `0x5444`, `0x5484` (SA; types unknown)      | unknown                                                           | —                                                             | —             | no                                               |

## Gimbal and framing

| Feature                                                                          | Command id(s)                                             | In Center UI           | obsbot4linux                                                                          | Capability | Verified on camera                       |
| -------------------------------------------------------------------------------- | --------------------------------------------------------- | ---------------------- | ------------------------------------------------------------------------------------- | ---------- | ---------------------------------------- |
| Zoom (arrows / slider)                                                           | `0x1942` (preview), V4L2 `zoom_absolute`                  | yes (CN)               | `zoom 1.0-4.0`                                                                        | `zoom`     | yes (CM)                                 |
| Field of view: Wide / Medium / Narrow                                            | short `04 01 v`                                           | yes (CN)               | `fov wide\|medium\|narrow`                                                            | `fov`      | yes (CM)                                 |
| Joystick (manual gimbal control)                                                 | `0x6484` speed, `0x0244` start/end                        | yes (CN)               | partial: `gimbal --pan --tilt` moves to an absolute pose (`0x6444`), no speed control | `gimbal`   | captured (CM)                            |
| Reset (gimbal to centre)                                                         | not recorded in the notes                                 | yes (CN)               | partial: `gimbal --pan 0 --tilt 0`, `zoom 1.0`                                        | `gimbal`   | unknown                                  |
| Coarse / fine joystick step                                                      | none: Center-side only                                    | yes (CN)               | not applicable                                                                        | —          | yes (CM: nothing is sent)                |
| Mirror image                                                                     | short `14 01 v`                                           | yes (CN)               | `mirror`                                                                              | `mirror`   | yes (CM)                                 |
| Invert view and gimbal (reverses pan direction only)                             | `0x3b84`                                                  | yes (CN)               | `invert`                                                                              | `invert`   | yes (CM)                                 |
| Initial position / start pose (set, go to, read)                                 | set `0x3ec4` (+ `0x3e44`), go `0x3904`, read `0x3884`     | yes (CN)               | `startup set\|go`                                                                     | `startup`  | set: captured; go and read: yes (CM)     |
| Initial state: also AI mode, speed, focus, exposure, white balance, image values | picture-settings block of `0x3e44`                        | yes (CN)               | partial: `startup set --ai` (AI mode only; rest sent cleared)                         | —          | captured (CM); block only partly decoded |
| Clear the start pose                                                             | `0x38c4` (SA)                                             | code only              | — (README: no way known)                                                              | —          | no                                       |
| Stop the gimbal                                                                  | `0x6704` (SA)                                             | code only              | —                                                                                     | —          | no                                       |
| Pose as Euler angles, speed variants                                             | `0x6404`, `0x64c4`, `0x6504`, `0x6544` (SA)               | code only              | —                                                                                     | —          | no                                       |
| Portrait (vertical) mode                                                         | short `0c 01 v` (SA; the UI text says the camera reboots) | unknown for this model | —                                                                                     | —          | no                                       |
| Image rotation                                                                   | `0x18c2`, `0x1882` (SA)                                   | unknown                | —                                                                                     | —          | no                                       |

## Presets and zones

| Feature                                                         | Command id(s)                                                                       | In Center UI                                                                                     | obsbot4linux                                | Capability | Verified on camera                             |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | ------------------------------------------- | ---------- | ---------------------------------------------- |
| Gimbal presets: add, rename, delete, recall (three slots)       | `0x3944`, `0x3a84`, `0x3984`, `0x39c4`; read selectors `0x0c`/`0x0d`                | yes (CN)                                                                                         | `preset save\|rename\|delete\|recall\|list` | `preset`   | captured (CM); list read verified              |
| Preset PTZ update                                               | `0x3e04` (CM) / `0x3a04` (SA)                                                       | yes (CN)                                                                                         | `preset save` on an existing slot           | `preset`   | captured (CM)                                  |
| Preset "more settings" (per-preset picture values)              | `0x3d84`                                                                            | yes (CN)                                                                                         | — (sent cleared on delete only)             | —          | captured (CM); block only partly decoded       |
| Preset as initial state                                         | `0x3ec4` source slot                                                                | yes (CN)                                                                                         | `startup set --preset`                      | `startup`  | captured (CM)                                  |
| Preset action and boot action                                   | `0x3d84`/`0x3dc4`, `0x3e44`/`0x3e84` (SA names)                                     | unknown (CM observes `0x3d84` as picture settings, so the SA names and the capture do not agree) | —                                           | —          | no                                             |
| Non-tracking areas (zone presets): add, rename, delete, trigger | `0x1384`, `0x1544`, `0x13c4`, `0x1604`, `0x1404`; read `0x1344`, `0x1444`, `0x1504` | yes (CN)                                                                                         | —                                           | —          | captured (CM); reads rejected while none exist |
| Camera-side presets                                             | `0xd042`, `0xd082`, `0xd0c2` (SA)                                                   | unknown (SA: model unknown)                                                                      | —                                           | —          | no                                             |

## Gestures

Gestures are write-only in obsbot4linux: `status` reports only the target-lock gesture.

| Feature                                                              | Command id(s)                                                                                            | In Center UI | obsbot4linux                   | Capability | Verified on camera |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ------------ | ------------------------------ | ---------- | ------------------ |
| Gesture: target lock                                                 | `0x30c4`                                                                                                 | yes (CN)     | `gesture set target-lock`      | `gesture`  | yes (CM)           |
| Gesture: zoom                                                        | `0x3144`                                                                                                 | yes (CN)     | `gesture set zoom`             | `gesture`  | yes (CM)           |
| Gesture: dynamic zoom                                                | `0x3344`                                                                                                 | yes (CN)     | `gesture set dynamic-zoom`     | `gesture`  | yes (CM)           |
| Gesture: direction change                                            | `0x33c4`                                                                                                 | yes (CN)     | `gesture set direction-change` | `gesture`  | yes (CM)           |
| Gesture zoom ratio                                                   | `0x3244`                                                                                                 | yes (CN)     | `gesture zoom-ratio`           | `gesture`  | yes (CM)           |
| Gesture record, generic gesture parameters, combined gesture control | `0x31c4`, `0x3444`/`0x3484`, `0x2044`/`0x2084` (payload size tells it from hand yaw), short `20 05` (SA) | unknown      | —                              | —          | no                 |

## Image

Rows marked "UVC" are plain V4L2 controls on the camera terminal / processing unit, reached with
`obsbot4linux image <name> <value>`; they all fall under the `image` capability.

| Feature                                                                  | Command id(s)                                                                        | In Center UI                | obsbot4linux                                                  | Capability | Verified on camera                                         |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------ | --------------------------- | ------------------------------------------------------------- | ---------- | ---------------------------------------------------------- |
| HDR on / off                                                             | short `01 01 v`                                                                      | yes (CN)                    | `hdr`                                                         | `hdr`      | captured (CM)                                              |
| Anti-flicker 50 Hz / 60 Hz / off                                         | UVC power line frequency                                                             | yes (CN)                    | `image power_line_frequency`                                  | `image`    | yes (UVC; read in status as `anti_flicker`)                |
| Autofocus on / off                                                       | UVC focus auto                                                                       | yes (CN)                    | `focus auto`                                                  | `focus`    | yes (CM)                                                   |
| Autofocus mode: Global / Face                                            | `0x3602` (receiver 2)                                                                | yes (CN)                    | `focus mode global\|face`                                     | `focus`    | yes (CM)                                                   |
| Manual focus slider                                                      | UVC focus absolute                                                                   | yes (CN)                    | `focus set 0-100`                                             | `focus`    | yes (CM)                                                   |
| Auto exposure on / off (manual)                                          | `0x2442` (+ UVC)                                                                     | yes (CN)                    | `exposure manual` / `global` / `face`                         | `exposure` | yes (CM)                                                   |
| Exposure metering: Global / Face                                         | short `03 01 v`                                                                      | yes (CN)                    | `exposure global\|face`                                       | `exposure` | yes (CM)                                                   |
| Shutter and ISO in manual exposure                                       | `0x2982`, UVC exposure time / gain                                                   | yes (CN)                    | `image exposure_time`, `image gain`                           | `image`    | yes (CM for shutter; ISO as UVC gain not separately noted) |
| Exposure compensation (EV bias)                                          | not identified (CN: set to +1.0 and back to 0.0); candidates `0x24c2`, `0x2582` (SA) | yes (CN)                    | —                                                             | —          | no                                                         |
| White balance: temperature, auto                                         | UVC white balance                                                                    | yes (CN)                    | `image white_balance_temperature`, `image white_balance_auto` | `image`    | yes (UVC)                                                  |
| White balance modes other than manual temperature and auto, offset, area | `0x2082`, `0x2a42`, `0x38c2` (SA)                                                    | unknown                     | —                                                             | —          | no                                                         |
| Contrast, saturation, sharpness, hue                                     | UVC                                                                                  | yes (CN)                    | `image contrast\|saturation\|sharpness\|hue`                  | `image`    | yes (UVC; survive a restart, CM)                           |
| Brightness, gamma, gain, backlight compensation                          | UVC                                                                                  | unknown (not clicked in CN) | `image brightness\|gamma\|gain\|backlight_compensation`       | `image`    | yes (UVC controls exist)                                   |
| Reset icons (image content, white balance, exposure, focus)              | none identified                                                                      | yes (CN)                    | — (set the defaults by hand)                                  | —          | unknown                                                    |
| ISO limit, AE lock, face AE, WDR, night mode, image style, gamma mode    | `0x2182`, `0x2202`, `0x2242`, `0x02c2`, `0x3042`, `0x3842`, `0x2f82` (SA)            | unknown                     | —                                                             | —          | no                                                         |

## Audio

| Feature                                         | Command id(s)                               | In Center UI | obsbot4linux      | Capability | Verified on camera |
| ----------------------------------------------- | ------------------------------------------- | ------------ | ----------------- | ---------- | ------------------ |
| Noise reduction: off / weak / medium / strong   | short `0a 01 v`                             | yes (CN)     | `audio noise`     | `audio`    | yes (CM)           |
| Automatic gain                                  | short `17 01 v`                             | yes (CN)     | `audio gain`      | `audio`    | yes (CM)           |
| Pickup distance: near / standard / far          | short `1b 01 v`                             | yes (CN)     | `audio distance`  | `audio`    | yes (CM)           |
| Microphone on while asleep                      | short `13 01 v`                             | yes (CN)     | `audio sleep-mic` | `audio`    | yes (CM)           |
| Disable microphone (camera restarts, ~18 s)     | short `1c 01 v`                             | yes (CN)     | `audio mic`       | `audio`    | yes (CM)           |
| Microphone volume, audio source, mute, VQE type | `0x8102`, `0x8942`, `0x8a02`, `0x8182` (SA) | unknown      | —                 | —          | no                 |
| Voice commands (KWS), audio mode                | short `23 02 a b`, `22 02` (SA)             | unknown      | —                 | —          | no                 |

## Device and system

| Feature                                         | Command id(s)                                              | In Center UI                       | obsbot4linux                    | Capability                | Verified on camera           |
| ----------------------------------------------- | ---------------------------------------------------------- | ---------------------------------- | ------------------------------- | ------------------------- | ---------------------------- |
| Sleep / wake (power button)                     | `0xa0c2`                                                   | yes (CN)                           | `sleep`, `wake`                 | `sleep`, `wake`           | yes (CM)                     |
| Auto-sleep time / off                           | short `0b 02 <i16>`                                        | yes (CN)                           | `auto-sleep`                    | `auto_sleep`              | yes (CM)                     |
| Sleep picture: black / built-in image           | short `0e 02 00 v`                                         | yes (CN)                           | `sleep-background black\|image` | `sleep_background`        | yes (CM)                     |
| Sleep picture: own image (upload)               | upload over the camera's file channel (details not public) | yes (CN)                           | —                               | —                         | yes (CM: seen from Center)   |
| Mirror the sleep picture                        | short `0e 02 02 v`                                         | yes (CN)                           | `sleep-background-mirror`       | `sleep_background_mirror` | yes (CM)                     |
| Status LED brightness / off                     | short `1a 01 v`                                            | yes (CN)                           | `led 0-3`                       | `led`                     | yes (CM)                     |
| LED blinks while a settings dialog is open      | short `18 01 v`                                            | yes (CN, internal)                 | —                               | —                         | yes (CM); not a user setting |
| Factory reset                                   | `0xa802`                                                   | yes (CN)                           | `factory-reset`                 | `factory_reset`           | yes (CM, also from Linux)    |
| Expert log export                               | prepare `0xaa02`/`0xaa42` (SA), then file transfer         | yes (CN)                           | —                               | —                         | no                           |
| Do not sleep while no app streams               | short `12 01 v` (SA)                                       | code only                          | —                               | —                         | no                           |
| AI mode at power-on                             | short `19 01 m` (SA)                                       | code only                          | —                               | —                         | no                           |
| Camera button mode and custom action            | short `07 01`, `08 03 a b c` (SA)                          | code only (button on other models) | —                               | —                         | no                           |
| USB / UVC mode, plug detection, system time     | `0xc402`, `0xa302`, `0xa082` (SA)                          | unknown                            | —                               | —                         | no                           |
| Scheduled sleep and resume                      | `0xa382`..`0xa442` (SA; probably network models)           | unknown                            | —                               | —                         | no                           |
| Tracking target readback (not a Center setting) | query `0x01c4`                                             | not applicable                     | `status` `target`               | `target`                  | yes                          |

## Firmware

| Feature                                     | Command id(s)                                                              | In Center UI | obsbot4linux              | Capability | Verified on camera |
| ------------------------------------------- | -------------------------------------------------------------------------- | ------------ | ------------------------- | ---------- | ------------------ |
| Firmware update from a local file           | `0x1848` (upgrade receiver), file transfer over the camera's MTP interface | yes (CN)     | — (not part of this repo) | —          | captured (CM)      |
| Online check for a newer firmware           | OBSBOT cloud endpoints (SA, strings only, never fetched)                   | code only    | —                         | —          | no                 |
| Firmware version display                    | status / upgrade queries                                                   | unknown      | —                         | —          | unknown            |
| Upgrade state and result, accessory upgrade | `0x0088`, `0x00c8`, `0x2008` (SA)                                          | unknown      | —                         | —          | no                 |

## Fine-grained capabilities

Today `CAPABILITIES` in `src/camera.rs` is one static list (28 names) that every detected camera
reports, whatever it can do. A per-element UI (hide a control the camera lacks) would need names
at the granularity of the controls, not of the command groups. Names a per-element UI would need,
none of which exist yet:

- **Tracking**: one per AI mode (`tracking_normal`, `tracking_upper_body`, `tracking_close_up`,
  `tracking_headless`, `tracking_lower_body`, `tracking_hand`, `tracking_group`), because the
  mode set differs per model; `speed_sport` if a model only has one speed.
- **Hand**: `hand_side`, `hand_area` (once settable), split from `hand_return`.
- **Gestures**: `gesture_target_lock`, `gesture_zoom`, `gesture_dynamic_zoom`,
  `gesture_direction_change`, `gesture_zoom_ratio` instead of one `gesture`.
- **Focus**: `autofocus`, `autofocus_mode` (global / face), `manual_focus`; today one `focus`.
- **Exposure**: `exposure_metering` (global / face), `exposure_manual`, `exposure_compensation`
  (once it exists), `shutter`, `iso`.
- **White balance**: `white_balance_auto`, `white_balance_temperature`.
- **Picture values**: one per UVC control the camera really exposes (`contrast`, `saturation`,
  `sharpness`, `hue`, `brightness`, ...) instead of the single `image`; the list could be read from
  the V4L2 control enumeration rather than hard-coded.
- **Anti-flicker**, **HDR**, **fov** (and which steps), **mirror**, **invert**.
- **Audio**: `audio_noise_reduction`, `audio_gain`, `audio_distance`, `audio_sleep_mic`,
  `audio_mic_disable`.
- **Device**: `led` (and whether brightness levels exist), `sleep_background_custom`, `auto_sleep`.
- **Gimbal**: `gimbal` (any pan/tilt at all, false on the Meet family), `gimbal_joystick`,
  `preset_slots` as a number rather than a flag, `zone_areas`.

The per-model matrix is still open. The static analysis only infers it ("Other OBSBOT models" in
the analysis): gimbal, presets, zone, hand and gestures on Tiny 2, Tiny 2 Lite, Tiny SE, Tiny 3
Lite, Tail Air, Tail 2 and Tail 2S; no gimbal on the Meet family; Center's per-model UI gating,
and whether the status block exists on other models, are not determined. Nothing was tested on
another model, so the matrix cannot be filled from evidence yet.

## Open questions

Rows marked `unknown` in the "In Center UI" column, or with unknown verification:

1. Composition lines (horizontal / vertical offset), auto horizontal framing and the target zoom /
   view types: does the Tiny 2 Lite UI show them, and where?
2. Exposure compensation: which command carries it? Center shows it; nothing in the captures has
   been mapped to an id.
3. The "reset" arrow (gimbal) and the four reset icons in the image panel: do they send commands
   or set defaults through the ordinary ones?
4. Preset action / boot action (`0x3d84`, `0x3dc4`, `0x3e44`, `0x3e84`): the static analysis names
   and the capture meaning of `0x3d84` disagree.
5. The picture-settings block of presets and of the initial state: the AI mode (only normal seen),
   sport speed, global autofocus, exposure parameters and white balance modes other than manual
   are not decoded.
6. Gesture record, generic gesture parameters and combined gesture control: UI use unknown; also
   whether the camera reports the gesture switches other than target lock.
7. Virtual track, auto framing, portrait mode, rotation: Tiny 2 Lite or other models only?
8. White balance modes beyond manual temperature and auto; ISO limit, AE lock, WDR, night mode,
   image style.
9. Audio beyond the five captured settings: volume, source selection, voice commands.
10. Camera button actions, "do not sleep without stream", "AI mode at power-on", USB mode: UI on
    this model?
11. Firmware version display and the online check: does Center show them for this model, and how
    is the version read?
12. Per-model UI gating in Center, which decides the per-model capability matrix.
