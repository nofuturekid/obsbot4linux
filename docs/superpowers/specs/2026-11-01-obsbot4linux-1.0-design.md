# obsbot4linux 1.0: rebuild design

Status: draft for review (2026-11-01). Decisions made with Thomas in the brainstorming
session of 2026-11-01.

## 1. Goal

obsbot4linux 1.0 is a rebuild that ends in a clean structure with no legacy baggage. It
is a rebuild on a branch with selective carry-over, not a migration in steps. The camera
knowledge we have earned (frame transport, command bytes, status parsing, captures,
byte-exact tests) is carried over. The daemon core, protocol, state model and CLI are
written new.

Success criteria:

- Every function of 0.13.x is still available (parity table, section 9), verified on the
  real Tiny 2 Lite. The one exception is the hidden `gimbal center`, which misbehaved in
  tests.
- Clients (CLI, Dux widget, the coming desktop app) talk one typed, versioned protocol
  ("protocol 2"). It is published as a JSON schema.
- Several cameras at once, hotplug, and stable device identity.
- A further OBSBOT model is added as a model module plus fixtures, with no client
  change.
- CI tests daemon, client and CLI without a camera (`--fake`).

Out of scope for 1.0: the data-driven command catalogue and models derived from the
Center code (milestone M4), the device lease for the firmware updater (M5), network
cameras (Tail Air, Tail 2: noted as a possibility only), and the desktop app (its own
repo and spec).

## 2. Principles

- **Clean over compatible.** The user base is very small. Breaking changes are made with
  a transition (release notes, minimum versions), not by carrying old structure. There
  is no v1 protocol adapter.
- **The camera is the truth.** Byte-exact frame tests and captures are kept: they
  encode what the camera understands, not our design.
- **The daemon owns the device.** No client touches the camera directly.
- **One contract.** The socket protocol, `--json` output and the schema are the same
  thing.
- **Camera features only.** No invented automation.
- **Nothing around the camera.** What the camera cannot do or cannot report is not
  there. The daemon keeps no state of its own beyond optional user-given names, and
  there is no key, no remembered settings and no migration.

## 3. Architecture

Cargo workspace, one repo, one release:

```
crates/obsbot-proto    protocol types (State, Patch, Action, Event, ErrorCode,
                       DeviceDescription), protocol version, schema generation.
                       Depends on serde only (+ schemars).
crates/obsbot-client   synchronous socket client.
crates/obsbot-device   transport (USB/UVC XU + ioctls; a seam for a later network
                       transport), trait Camera, models/{tiny2lite, fake}, registry by
                       USB id.
crates/obsbot4linuxd   daemon binary.
crates/obsbot4linux    CLI binary (a pure client).
packaging/             arch, deb, rpm, systemd, completions
docs/                  protocol-2.md, field-ledger.md
schema/protocol-2.json generated; CI fails if it is stale
```

Dependency direction: proto ← client ← CLI; proto ← device ← daemon. The CLI does not
depend on device code.

The updater (`shared/obsbot4linux-updater`, private on Forgejo) stays outside for now.
Whether it moves into the public workspace is decided with M5.

### 3.1 Carry-over per module (0.13.x → 1.0)

| 0.13.x                                           | 1.0                                              | Decision                                |
| ------------------------------------------------ | ------------------------------------------------ | --------------------------------------- |
| `frame.rs` (frame, CRC, answer parser, sequence) | `obsbot-device/transport`                        | carry over                              |
| ioctl/XU access in `device.rs`, `camera.rs`      | `obsbot-device/transport`                        | carry over, regroup                     |
| `commands.rs` byte builders + setting enums      | `obsbot-device/models/tiny2lite`                 | carry over                              |
| `status.rs`, `presets.rs` parsers                | `obsbot-device/models/tiny2lite`                 | carry over                              |
| `streaming.rs` (kernel probe + names)            | `obsbot-device`                                  | carry over                              |
| `models.rs` (detection)                          | `obsbot-device/registry`                         | rewrite (serial-based identity)         |
| `camera.rs` sequencing (Tiny 2 Lite flows)       | `models/tiny2lite` behind `trait Camera`         | carry over the logic, rewrite the shape |
| `daemon.rs`, `proto.rs`, `client.rs`             | `obsbot4linuxd`, `obsbot-proto`, `obsbot-client` | rewrite                                 |
| `cli.rs`                                         | `obsbot4linux`                                   | rewrite                                 |
| `state.rs` (remembered values)                   | –                                                | drop (everything is read from the camera) |
| byte-exact tests, captures                       | `obsbot-device` tests + fixtures                 | carry over                              |

Every carried-over file keeps its licence header. Files derived from Tiny4Linux keep the
EUPL Article 5 note.

## 4. Protocol 2

The full client reference (per-message schemas, a worked optimistic-UI sequence,
lifecycle diagrams, a complete multi-device transcript) is written as
`docs/protocol-2.md` in stage 1, from this section. The field-by-field evidence for the
state model (command, readback, source, verification level, asleep behaviour) is kept in
`docs/field-ledger.md`.

### 4.1 Messages

JSON lines over the Unix socket `$XDG_RUNTIME_DIR/obsbot4linux.sock` (mode 0600;
override `OBSBOT4LINUX_SOCKET`). One JSON object per line, UTF-8, at most 64 KiB; a
longer line closes the connection.

**Requests** carry `id` (integer or string, echoed verbatim) and exactly one verb:

| Verb    | Shape                                                                                              | Reply `ok`                                                                                                           |
| ------- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `hello` | `{"id","hello":{"protocol":2,"client":"<name version>","subscribe":bool=true,"live":["pose"]=[]}}` | `{"protocol":2,"daemon":"1.0.0","epoch":"<random>","devices":[<Device>…],"states":{"<device id>":{"rev","state"}…}}` |
| `get`   | `{"id","device","get":["<path>"…]\|null}`                                                          | `{"rev","state"}`, the full state or only the paths asked for                                                        |
| `set`   | `{"id","device","set":<patch>}`                                                                    | `{"rev"}`                                                                                                            |
| `do`    | `{"id","device","do":"<action>","with":{…}}`                                                       | `{"rev","result":{…}?}`                                                                                              |
| `live`  | `{"id","live":["pose"…]}`                                                                          | `{}`. Replaces the connection's live list (e.g. a widget opens or closes its popup)                                  |

**Events** (only on connections with `subscribe:true`):

- `{"event":"state","device","rev","state"}`: the full state of one device.
- `{"event":"devices","devices":[<Device>…]}`: the full device list, sent whenever an
  entry is added, its `available` flips, its name changes, or its ID is replaced.

**Errors:** `{"id","error":{"code","message","rev"?,"applied"?,"failed"?,"supported"?}}`.
`message` is always present (English, for logs). `rev` is present whenever the daemon
re-read the device before answering.

| Code             | When                                                                                                                                                 | Extra fields                                            |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| `protocol`       | first message not `hello`, or an unsupported version (connection closed; `id:null` if there was none)                                                | `supported:[2]`                                         |
| `invalid`        | malformed request, no or two verbs, unknown action, unknown path, wrong type, read-only path, duplicate device name                                  | –                                                       |
| `unknown_device` | no such device ID                                                                                                                                    | –                                                       |
| `unsupported`    | the model has no such setting or action                                                                                                              | –                                                       |
| `out_of_range`   | value outside the description                                                                                                                        | –                                                       |
| `unavailable`    | device unplugged                                                                                                                                     | –                                                       |
| `asleep`         | the camera ignores this setting or action while asleep                                                                                               | –                                                       |
| `busy`           | 32 requests queued for the device, or the camera is in a blocking operation (factory reset, the restart after `audio.enabled=false`, later `leased`) | –                                                       |
| `camera`         | the camera refused or did not answer within the transport timeout carried over from 0.13                                                             | `rev`, `applied:[paths]`, `failed:"<path>"` (for `set`) |
| `internal`       | a daemon bug                                                                                                                                         | –                                                       |

**Ordering and `rev` (normative):**

1. `rev` is per device. It only goes up and restarts with the daemon. `hello` returns a
   random `epoch` per daemon start. A client that reconnects and sees a new epoch drops
   every `rev` it is still waiting for.
2. Replies for one device come in request order. Requests for different devices may be
   answered in any order. Requests may be pipelined.
3. For a `set` or `do` that changes state, the daemon writes the state event to every
   subscribed connection, the requester included, **before** the reply. The reply's
   `rev` is the rev of the first state that reflects the request. If nothing changed
   (same value, or a `no_effect` setting), it is the current rev.
4. A client with optimistic UI shows its value until a state with `rev ≥ reply.rev`
   arrives, then shows the state. There are no hold timers.
5. `set`: the whole patch is validated first, so if any field is invalid, nothing
   changes. The worker then applies the fields in an order the model defines as safe
   (for example `exposure.anti_flicker` before `exposure.mode=manual`, and
   `exposure.mode=manual` before `exposure.time`/`gain`; see 0.13 `manual_exposure`).
   On a camera failure midway it stops, re-reads, publishes the real state, and answers
   `camera` with `applied`/`failed`. There is no rollback, because the camera has no
   transactions.
6. Motion actions (`gimbal.move`, `gimbal.turn`, `zoom.move`, `preset.recall`,
   `view.go`, `zone.go_return`, `hand.go_return`, `power_on.go`) reply when the camera
   has accepted the command. State always shows **measured** values (pose, zoom factor),
   so it moves through later events. Whether the camera reports the zoom target or the
   current zoom during a speed-limited `zoom.move` is checked in stage 2, and
   `protocol-2.md` documents the result.
7. Protocol versions: additive changes (fields, actions, models, enum values) keep
   `protocol: 2`. Clients ignore unknown fields and show unknown enum values as raw text.
   Breaking changes raise the number.

**Reconnect (recommended client behaviour):** reconnect with a backoff from 0.5 s
doubling up to 5 s. Socket activation starts the daemon. Re-send `hello` and rebuild
from its reply.

### 4.2 Devices

```
Device = { id, name, serial, model, firmware, video_node, available,
           replaced_by?, settings{<leaf path> → Setting}, actions{<name> → Action},
           limits{presets: 3, views: N} }

Setting = { type: bool | enum | number | range | pose | point | box | string | list,
            label (English, for generic UIs),
            values[] + writable[] (enum: all values a state may show / the ones a set accepts),
            min, max, step, unit, precision (number; range bounds),
            kind: digital|optical|hybrid, optical_max (zoom.factor only),
            max_len (string), readonly, nullable,
            readback: true | false (false = the camera accepts it but cannot report it:
                     state shows the value written since the daemon started, else null),
            asleep: accepted | ignored | unknown,
            no_effect (accepted by the camera, no observable effect on this model) }

Action = { label, params{<name> → Setting}, result{<name> → Setting}?, asleep }
```

- Settings are described per **leaf** path (`zone.area.pan`, `led.brightness`). Only
  leaves are writable. A patch may still nest them (`{"zone":{"area":{"pan":…}}}`).
- **Absent vs null:** a field the model does not have is absent from state and
  description. `null` appears only where the description says `nullable`.
- **Shapes** are always objects: range `{min,max}`, pose `{pan,tilt,zoom}`, point
  `{x,y}`, box `{left,top,right,bottom}`.
- **Units and frames:** angles in degrees, pan positive = camera turns to its left, tilt
  positive = up, 0/0 = straight ahead. Picture coordinates 0..1 from the top-left corner.
  Zoom as a factor (1.0 = wide). Gimbal speeds in °/s. Durations in seconds (precision
  0.1). `exposure.time` in microseconds (the camera steps in 100 µs). Colour temperature
  in K. Percentages 0–100. Precision per setting in the description (angles 0.1, zoom
  0.01, fractions 0.01).
- **Identity:** The Tiny 2 Lite has no USB `iSerial`, so two identical units collide in
  `/dev/v4l/by-id`. The camera serial is read with query `0x18c8` to receiver `0x0d`
  (`getDevSnR`, 14 ASCII bytes; Center asks it at every connect; verified live
  2026-11-01). The ID is `sn:<serial>`. The query is retried (3 tries over about 10 s,
  also after re-enumeration or wake) before the fallback `port:<usb path>`. A `port:` ID
  is never stored. When it later resolves to an `sn:` ID, the `devices` event shows the
  old entry once with `replaced_by` and then drops it. Clients move their selection. Whether `0x18c8` answers while asleep is
  checked in stage 2.
- **Unplug:** the entry stays with `available:false` while the daemon runs. One last state
  event carries `available:false` and the last known values. No more state events are
  sent until it is back.
- **Names** are optional and unique per installation (`device.rename` with a used name
  gives `invalid`). Without a name, UIs show the model plus the serial's last 4
  characters.
- **Serial exposure:** UIs show the serial openly. Logs, the journal and CLI `--json`
  shorten it (`sn:RMOW…XYZ1`). `devices --show-serial` shows it in full. The protocol
  carries the full ID.
- **Never sent, never in any catalogue:** `0x1888` (writes the serial to OTP) and
  `0x1ac8` (runs a shell command), both on receiver `0x0d`.

### 4.3 State model

✎ = writable leaf. Types, ranges, values, units and asleep behaviour come from the device
description. The evidence per field is in `docs/field-ledger.md`.

```
available, asleep
on_air     { active, apps[] }                    apps: process names, best effort
gimbal     { pan, tilt, invert✎ }                pan/tilt measured; moved only by actions
zoom       { factor✎, field_of_view✎ }           fov values wide|medium|narrow|custom,
                                                 writable wide|medium|narrow
tracking   { mode✎, profile✎, speed✎, target { detected, box|null } }
             mode:    static|normal|upper_body|close_up|headless|lower_body|hand|group
             profile: standard|headroom|sport    (Center: "tracking speed")
             speed:   lazy|slow|normal|fast|very_fast|auto
framing    { auto_zoom✎, size✎, zoom_speed✎, offset { x✎, y✎ }, part✎ }
             size: long_shot … big_close_up; zoom_speed: slow|medium|fast;
             part: normal|head_removal|lower_body
gestures   { target_lock✎, zoom✎, zoom_ratio✎, dynamic_zoom✎, direction_change✎ }
zone       { enabled✎, follow✎, area { pan✎, tilt✎ }, return <pose> }
hand       { side✎ left|right, area { pan✎, tilt✎ }, axes { pan✎, tilt✎ },
             return_after✎, move_gimbal✎ (readback false), return <pose>|null }
presets    [ { slot, name, pose } ]
views      [ { id, name, pose } ]
exposure   { mode✎ auto|manual, region✎ face|global, time✎, gain✎, hdr✎,
             anti_flicker✎ off|50hz|60hz|auto }
image      { brightness✎, contrast✎, saturation✎, hue✎, sharpness✎, gamma✎,
             backlight_compensation✎, white_balance { auto✎, temperature✎ },
             mirror✎, flip✎ (only models that offer it), frame_rate }
focus      { mode✎ continuous|single|manual, region✎ face|global, position✎ }
audio      { enabled✎, volume✎, noise_reduction✎ off|weak|medium|strong,
             auto_gain✎, in_sleep✎ }
led        { brightness✎, blink✎ (readback false) }
sleep      { auto { enabled✎, seconds✎ }, background✎, background_mirror✎,
             wake_zoom✎ (no_effect on the Lite) }
             background values black|image|own_image, writable black|image
power_on   { mode✎, start { source, pose, mode✎ }, settings }
```

**`power_on`** holds two different camera settings that 0.13 exposed as `wake mode` and
`startup`:

- `power_on.mode`: the tracking mode the camera starts in at power-on (short `19`, all
  modes, status byte `0x20`; 0.13 `wake mode`, verified by replug tests, #102).
- `power_on.start`: the initial-state block (`0x3ec4` pose source + `0x3e44` block,
  readback `0x3884`/`0x3e84`; 0.13 `startup`).
  - `source` is `own` or a preset slot.
  - `pose` is read-only and set through actions.
  - `mode` is the block's AI mode, nullable: `null` is "not set", which is different
    from `static`; values static|normal|upper_body|headless|group.
- `power_on.settings`: the block's other fields (AI-related, autofocus, exposure, white
  balance, contrast…hue, as decoded in 0.13 `presets.rs`), read-only. Changing the pose
  keeps them; the model rewrites the block as 0.13 `startup_with_ai` does.

**Removed from 0.13.x output** (nothing that a user could set is lost; see section 9):

- `capabilities` and `limits`: both are in the description now.
- `status`, the raw camera block, and `state`, the daemon memory block.
- `model_info`, `digital_zoom_percent`, `sleep_images`, and the raw V4L2 fields.
- `device`, the device path: now `video_node`.
- `streaming`: now `on_air`.

The following duplicates are merged:

- `focus.auto` (UVC continuous AF) is the same camera bit as `focus.mode=continuous`.
- `audio.pickup` (near/standard/far) is only `audio.volume` quantised (below 50 near,
  50–84 standard, from 85 far). UIs may offer those three as volume presets.

**Open points to verify live in stage 2** (each is either added or dropped, and the
ledger records the result):

- `image.flip` via `0x6944`: untested. Offered only if it works.
- Resetting the start pose with `0x38c4` (would become `power_on.reset`), and resetting
  the zone area with `0x4f84`–`0x5044`.
- The layout of `tracking.target.box`: 0.13 documents left/top/right/bottom, but the
  inventory says the layout is not decoded.
- Asleep behaviour of `zoom.factor`, of `0x18c8`, and of the `0x80c2` and `0x3642`
  queries (no answer while asleep, so `audio.volume` and `focus.*` are unknown then).
- Exposure compensation (Center): likely UVC `backlight_compensation`. This is a guess.

### 4.4 Actions

```
gimbal.move {pan, tilt, speed?}        gimbal.step {pan, tilt}        gimbal.stop
gimbal.turn {pan_speed, tilt_speed, keep?}
zoom.move {factor, speed}              zoom.stop
preset.save {slot, name?, pose?}       preset.recall {slot}           preset.rename {slot, name}
preset.delete {slot}
view.save {name?, pose?} → {id}        view.go {id}                   view.rename {id, name}
view.delete {id}
zone.return_here                       zone.go_return
hand.return_here                       hand.go_return                 hand.return_clear
hand.area_reset
focus.point {x, y}
power_on.set_pose {pose}               power_on.use_current           power_on.use_preset {slot}
power_on.go
sleep                                  wake
image.reset                            factory_reset {confirm: true}
device.rename {name}
```

- `gimbal.step` and `image.reset` have no CLI command in 0.13.x. They exist for widget
  parity (the step buttons and "Reset to defaults").
- `gimbal.turn` keeps turning until `gimbal.stop` or speed 0. It stops when the issuing
  connection closes, unless `keep:true`; the CLI sends `keep:true` so a one-shot call
  works, as in 0.13 `gimbal rate`. It always stops on sleep, unplug and daemon shutdown.
- `preset.save`/`view.save` without `pose` use the live pose (`0x0043`).
- `gimbal.center` (0.13, hidden) is **not** carried over: it was hidden because it
  misbehaved in tests.

## 5. Daemon (obsbot4linuxd)

- Synchronous threads, no async runtime: an acceptor, one thread per client, one worker
  per device (the only code that touches that device), and a udev monitor.
- **Write path:** validate against the description (→ `invalid` / `out_of_range` /
  `unsupported` / `asleep`), send to the camera, read back the affected group (a `readback:false` value is
  confirmed once the camera accepted it), then bump `rev`, broadcast, and reply (ordering
  rules in 4.1). Only confirmed state is published.
- **While asleep** the worker keeps polling status only. State values it cannot read
  then keep their last known value. `do wake` replies once status reports awake.
- **Poll tiers** (carried over from 0.13 `daemon.rs`):
  - every 1 s: status (asleep, mode, target box, on air). The live pose (`0x6604`) is
    read in this tier **only while at least one connected client has `"pose"` in its live
    list** (`hello` or `live`), because each pose query holds the camera lock (#68).
    Otherwise the pose changes with the 5 s read.
  - every 5 s and after each command: the full settings (all groups except the next
    tier).
  - every 60 s and after a related command: zone, hand, power-on and presets/views
    (slow multi-frame reads).
  - An unchanged state produces no event and no new `rev`.
- **Hotplug:** On plug-in, read model and serial, derive the ID, emit `devices`. On
  unplug the device stays listed with `available:false`. On re-plug it is matched by ID.
- **Asleep:** the description says per setting whether the camera accepts it while
  asleep. Measured in 0.13: FOV, zoom and `power_on.go` are ignored, most others are
  kept. Zoom is re-checked in stage 2. An ignored setting is refused with `asleep`;
  `unknown` is passed through.
- **No state file.** Everything is read from the camera. The only file is
  `$XDG_STATE_HOME/obsbot4linux/names.json` (serial → name), and it exists only once a
  camera was renamed. It is written atomically (temp file + rename).
- **0.13 leftovers:** `state.json` held only the last gimbal pose and the exposure mode,
  and both are now read from the camera (live pose `0x0043`, status `0x07`). 1.0
  deletes it on first start, and the release notes say so. Nothing is imported or lost.
- **Single instance:** an `flock` on `$XDG_RUNTIME_DIR/obsbot4linux.lock`. A second
  daemon exits with a clear message.
- **Operation:** The unit names stay `obsbot4linux.socket` / `obsbot4linux.service`, so
  existing enablements keep working with no transition. Only `ExecStart` changes to
  `obsbot4linuxd`. Socket path unchanged. The daemon stays running.
- **Hardening:** `NoNewPrivileges=yes`, `ReadWritePaths=%S/obsbot4linux`, plus
  `ProtectSystem=strict` and `DeviceAllow` only if they work in a **user** unit without
  breaking `video`-group access or the udev monitor. No `PrivateNetwork` (it would cut
  off hotplug uevents). Acceptance is the real unit on Arch and on Ubuntu 24.04: the
  daemon starts, sees the camera, handles hotplug, and writes its state. A unit file that
  only parses does not count. Lines that fail are dropped and documented.
- **Logs** go to stderr/journal with serials shortened.
- **Fake:** `obsbot4linuxd --fake N` simulates N Tiny 2 Lite cameras: plausible state,
  `set`/`do` change it, hotplug and sleep can be triggered over a test-only action set
  that is enabled only with `--fake`. Real devices are not touched in fake mode.
- **Reserved:** device state `leased` for the updater (M5). It is not implemented in 1.0.

## 6. Client and CLI

### 6.1 obsbot-client

```rust
let mut c = Client::connect()?;                 // hello, protocol check
c.devices();                                    // &[DeviceDescription]
c.get(&id, None)?;                              // -> (Rev, State)
c.set(&id, Patch::new().zoom_factor(1.5))?;     // -> Rev
c.act(&id, Action::PresetRecall { slot: 1 })?;  // -> (Rev, Option<ActionResult>)
c.live(&["pose"])?;
for ev in c.events() { … }                      // Event::State / Event::Devices
```

Errors are typed (`ErrorCode`). There is no built-in retry: on disconnect the event
iterator ends with an error, and the caller decides whether to reconnect.

### 6.2 CLI (obsbot4linux)

The CLI is generic only. It maps 1:1 onto protocol 2, so new models and settings need no
CLI change. There are no per-feature subcommands and no shortcuts (users may define shell
aliases). `--direct` is removed: socket activation starts the daemon.

```
obsbot4linux status [--json]
obsbot4linux get <path>
obsbot4linux set <path>=<value> [<path>=<value> …]      # one patch
obsbot4linux do <action> [key=value …]
obsbot4linux describe [<group|path>]
obsbot4linux devices [rename <dev> <name>] [--show-serial]
obsbot4linux watch                                      # events as JSON lines
  --device <name|id>     needed only when more than one camera is connected
```

- Value syntax, one per setting type: bool `on|off|true|false`; enum option name
  (`upper_body`, also `upper-body`); number `1.5`; range `min,max` (only on leaf paths
  such as `zone.area.pan=-20,20`); pose `pan,tilt,zoom`; string as given (quote for the
  shell); point `x,y`; `none` = null (`power_on.start.mode=none`). Only leaves are set.
  Action parameters use the same syntax (`do view.save pose=0,5,1.2`).
- Human output: grouped, with units, serial shown. `--json` prints exactly the protocol
  objects.
- Exit codes: 0 ok, 1 internal, 2 invalid input (`invalid`, `out_of_range`,
  `unsupported`, `unknown_device`), 3 device not usable (`unavailable`, `asleep`,
  `busy`, `camera`, and no camera connected at all, e.g. `devices` with an empty list),
  4 daemon unreachable or wrong protocol.
- One-shot commands send `hello` with `subscribe:false` and read with `get`. Only
  `watch` subscribes.
- Completions for bash and zsh (clap_complete). Commands, paths, options and actions are
  completed **statically from the generated schema** of the Tiny 2 Lite and fake models,
  so Tab never activates the daemon. Only `--device` values are completed live, and only
  when the daemon is already running. Completions ship in all packages.
- `--device` matches the name first, then the ID. A name that matches nothing fails
  with the device list.

## 7. Multiple cameras

The daemon handles any number of cameras. Mixed models are not a special case, since each
device brings its own description. The CLI needs `--device` only when more than one
camera is connected; without it, it fails with the device list. Both widget and app get
a device picker. Tests run with `--fake 2`.

## 8. Adding a model

A model is one module in `obsbot-device/models/` that implements `trait Camera`:
description (settings, ranges, options, actions, asleep behaviour), encoding (reusing the
shared transport) and status decoding. It is added to the registry with its USB id,
with capture fixtures for byte tests. There is no cargo feature per model until there is
a real reason, such as size or a dependency. Models known only from
the Center code are marked **UNTESTED** in description, docs and release notes until an
owner confirms them. Clients need no change. The data-driven catalogue (TOML) comes with
M4 and the first derived model.

## 9. Parity table (1.0 acceptance)

Every 0.13.x command and option maps to a 1.0 path. Release only when every row has been
verified on the real camera with `scripts/check-parity.sh`: set → read back → restore.

| 0.13.x                                                                  | 1.0                                                                                                           |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `status [--json]`                                                       | `status [--json]`                                                                                             |
| `detect` (exit 1 without camera)                                        | `devices` (exit 3 without camera)                                                                             |
| `watch`                                                                 | `watch`                                                                                                       |
| `daemon`                                                                | `obsbot4linuxd`                                                                                               |
| `--direct`                                                              | removed (socket activation)                                                                                   |
| `live-pose on\|off`                                                     | `hello`/`live` with `live:["pose"]`                                                                           |
| `sleep` / `wake` (no subcommand)                                        | `do sleep` / `do wake`                                                                                        |
| `wake zoom F`                                                           | `set sleep.wake_zoom=F`                                                                                       |
| `wake mode M`                                                           | `set power_on.mode=M`                                                                                         |
| `tracking MODE`                                                         | `set tracking.mode=MODE`                                                                                      |
| `speed standard\|headroom\|sport`                                       | `set tracking.profile=…`                                                                                      |
| `speed-mode …`                                                          | `set tracking.speed=…`                                                                                        |
| `framing auto-zoom on\|off`                                             | `set framing.auto_zoom=…`                                                                                     |
| `framing size S`                                                        | `set framing.size=S`                                                                                          |
| `framing zoom-speed S`                                                  | `set framing.zoom_speed=S`                                                                                    |
| `framing offset --x --y`                                                | `set framing.offset.x=… framing.offset.y=…`                                                                   |
| `framing part P`                                                        | `set framing.part=P`                                                                                          |
| `hdr on\|off`                                                           | `set exposure.hdr=…`                                                                                          |
| `exposure global\|face`                                                 | `set exposure.mode=auto exposure.region=…`                                                                    |
| `exposure manual --time --gain`                                         | `set exposure.mode=manual exposure.time=… exposure.gain=…` (time now in µs)                                   |
| `zoom F [--speed S]`                                                    | `set zoom.factor=F` / `do zoom.move factor=F speed=S`                                                         |
| `zoom stop`                                                             | `do zoom.stop`                                                                                                |
| `gimbal --pan --tilt [--speed]`                                         | `do gimbal.move pan= tilt= [speed=]`                                                                          |
| `gimbal stop`                                                           | `do gimbal.stop`                                                                                              |
| `gimbal rate --pan --tilt`                                              | `do gimbal.turn pan_speed= tilt_speed=` (CLI sends `keep:true`)                                               |
| `gimbal center X Y` (hidden)                                            | **not carried over** (misbehaved in tests)                                                                    |
| `fov wide\|medium\|narrow`                                              | `set zoom.field_of_view=…`                                                                                    |
| `led N`                                                                 | `set led.brightness=N`                                                                                        |
| `led blink on\|off`                                                     | `set led.blink=…`                                                                                             |
| `mirror on\|off`                                                        | `set image.mirror=…`                                                                                          |
| `invert on\|off`                                                        | `set gimbal.invert=…`                                                                                         |
| `auto-sleep off\|N`                                                     | `set sleep.auto.enabled=… sleep.auto.seconds=N`                                                               |
| `sleep-background black\|image`                                         | `set sleep.background=…`                                                                                      |
| `sleep-background-mirror on\|off`                                       | `set sleep.background_mirror=…`                                                                               |
| `audio noise L`                                                         | `set audio.noise_reduction=L`                                                                                 |
| `audio gain on\|off`                                                    | `set audio.auto_gain=…`                                                                                       |
| `audio distance near\|standard\|far`                                    | `set audio.volume=…` (near < 50, standard 50–84, far ≥ 85)                                                    |
| `audio volume N`                                                        | `set audio.volume=N`                                                                                          |
| `audio sleep-mic on\|off`                                               | `set audio.in_sleep=…`                                                                                        |
| `audio mic on\|off`                                                     | `set audio.enabled=…`                                                                                         |
| `gesture set target-lock\|zoom\|dynamic-zoom\|direction-change on\|off` | `set gestures.<name>=…`                                                                                       |
| `gesture zoom-ratio R`                                                  | `set gestures.zoom_ratio=R`                                                                                   |
| `hand right\|left`                                                      | `set hand.side=…`                                                                                             |
| `hand return-here` / `return-go` / `return-reset`                       | `do hand.return_here` / `hand.go_return` / `hand.return_clear`                                                |
| `hand area --pan --tilt`                                                | `set hand.area.pan=min,max hand.area.tilt=min,max`                                                            |
| `hand area-reset`                                                       | `do hand.area_reset`                                                                                          |
| `hand pan\|tilt on\|off`                                                | `set hand.axes.pan=…` / `hand.axes.tilt=…`                                                                    |
| `hand return-after S`                                                   | `set hand.return_after=S`                                                                                     |
| `hand gimbal on\|off`                                                   | `set hand.move_gimbal=…`                                                                                      |
| `zone on\|off`                                                          | `set zone.enabled=…`                                                                                          |
| `zone follow on\|off`                                                   | `set zone.follow=…`                                                                                           |
| `zone area --pan --tilt`                                                | `set zone.area.pan=min,max zone.area.tilt=min,max`                                                            |
| `zone return-here` / `return-go`                                        | `do zone.return_here` / `zone.go_return`                                                                      |
| `zone view list`                                                        | `get views`                                                                                                   |
| `zone view save [--pan --tilt --zoom]` / `rename` / `delete` / `go`     | `do view.save` / `view.rename` / `view.delete` / `view.go`                                                    |
| `focus mode face\|global`                                               | `set focus.region=…`                                                                                          |
| `focus auto on\|off`                                                    | `set focus.mode=continuous\|manual`                                                                           |
| `focus af continuous\|single\|manual`                                   | `set focus.mode=…`                                                                                            |
| `focus point X Y`                                                       | `do focus.point x= y=`                                                                                        |
| `focus set N`                                                           | `set focus.position=N`                                                                                        |
| `startup set --pan --tilt --zoom`                                       | `do power_on.set_pose pose=pan,tilt,zoom`                                                                     |
| `startup set --preset N`                                                | `do power_on.use_preset slot=N`                                                                               |
| `startup set --current`                                                 | `do power_on.use_current`                                                                                     |
| `startup set --ai M\|none`                                              | `set power_on.start.mode=M\|none`                                                                             |
| `startup show`                                                          | `get power_on`                                                                                                |
| `startup go`                                                            | `do power_on.go`                                                                                              |
| `image CONTROL VALUE`                                                   | `set image.<control>=VALUE` (incl. `gamma`; exposure, focus and anti-flicker controls are under their groups) |
| `preset list`                                                           | `get presets`                                                                                                 |
| `preset save N [--name --pan --tilt --zoom]`                            | `do preset.save slot=N [name= pose=]`                                                                         |
| `preset rename` / `delete` / `recall`                                   | `do preset.rename` / `preset.delete` / `preset.recall`                                                        |
| `factory-reset --yes`                                                   | `do factory_reset confirm=true`                                                                               |

## 10. Way of working

- Long-lived branch `rebuild` in this repo. Each build stage is an issue with a PR into
  `rebuild`, with the `required` gate and an Opus review. At the end a single PR goes
  from `rebuild` to `main`, then tag `v1.0.0`.
- `main` stays at 0.13.x until then (patches only if needed).
- Build stages:
  1. Workspace, `obsbot-proto` (types, schema, golden files), fake model.
  2. `obsbot-device`: transport, `trait Camera`, `models/tiny2lite` (byte tests green,
     live check).
  3. `obsbot4linuxd` + `obsbot-client` (integration tests against `--fake`).
  4. CLI, completions, packaging (workspace: deb metadata moves from the root
     `Cargo.toml` to the binary crates; the release workflow builds both binaries; the
     udev dependency is checked for the aarch64 cross build), units (names unchanged,
     new `ExecStart`), hardening tested on the real unit.
  5. Parity script on the real camera, mutation tests (`obsbot-device`, `obsbot-proto`),
     docs (`protocol-2.md`, migration notes), widget switch, release.
- Before tagging: `HEAD..origin/main` must be empty.

## 11. Testing

| Level           | What                                                                            | Where                 |
| --------------- | ------------------------------------------------------------------------------- | --------------------- |
| Camera bytes    | byte-exact frame tests, captures as fixtures                                    | `obsbot-device`, CI   |
| Protocol        | golden files for protocol 2 (hello, state, errors) validated against the schema | `obsbot-proto`, CI    |
| Daemon + client | `--fake 2`: set/do, rev ordering, hotplug, asleep, error codes, multi-device    | CI                    |
| CLI             | output snapshots and exit codes against `--fake`                                | CI                    |
| Real camera     | `scripts/check-parity.sh`, every row of section 9                               | manual before release |

Golden files cover protocol 2 only. Intentional changes update them in the same PR.

## 12. Release and clients

- Package `obsbot4linux` (both binaries, units, completions, schema). Formats as today:
  Arch, deb, rpm and tarball for x86_64 and aarch64. The Forgejo mirror stays.
- CHANGELOG with a migration guide (old command → new path, `state.json` import, and
  what to set again if the import did not apply). The unit names are unchanged, so
  nothing needs re-enabling.
- **Dux widget 1.0** is released the same day. It uses protocol 2 (`hello`, device
  picker, `set`/`do`) and settles optimistic values by `rev`, replacing the up-to-3 s
  hold (#89). `plugin.json` requires obsbot4linux ≥ 1.0. Update the registry entry.
- **Desktop app** (`obsbot4linux-desktop`, own repo, own spec): built on
  `obsbot-proto` + `obsbot-client` via a git tag. Its shell can start once stage 1 and
  the fake daemon are on `rebuild`.

## 13. Later (not 1.0)

- M4: data-driven command catalogue (TOML embedded at build time, quirks in code) plus
  the first Center-derived model (UNTESTED).
- M5: device lease for the firmware updater; decide whether the updater joins the
  public workspace.
- Network transport (Tail Air / Tail 2): a possibility only.
- Async client, if a client needs it.
