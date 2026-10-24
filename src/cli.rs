// SPDX-License-Identifier: GPL-3.0-or-later

//! Argument parsing and output. Every command prints text, or JSON with `--json`.

use std::io::Write;

use clap::{Parser, Subcommand, ValueEnum};
use obsbot4linux::camera::Camera;
use obsbot4linux::commands::*;
use obsbot4linux::error::{Error, Result};
use obsbot4linux::presets::Pose;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "obsbot4linux",
    version,
    about = "Control the OBSBOT Tiny 2 Lite"
)]
pub struct Cli {
    /// Print machine-readable JSON (errors too, as {"ok":false,"error":...})
    #[arg(long, global = true)]
    pub json: bool,
    /// Do not use a running daemon; talk to the camera directly
    #[arg(long, global = true)]
    direct: bool,
    #[command(subcommand)]
    pub command: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl OnOff {
    fn get(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the daemon: owns the camera, serves the socket, broadcasts state changes
    Daemon,
    /// Stream state events as JSON lines and forward JSON requests from stdin (for widgets)
    Watch,
    /// Show all settings (the camera's status block, tracking readback, zoom, image controls, remembered state)
    Status,
    /// Put the camera to sleep
    Sleep,
    /// Wake the camera
    Wake,
    /// AI tracking mode
    Tracking { mode: AiMode },
    /// Tracking speed
    Speed { speed: TrackingSpeed },
    /// HDR
    Hdr { state: OnOff },
    /// Exposure: automatic metering over the whole picture (global) or on faces (face), or manual (then set exposure_time / gain with `image`)
    Exposure { mode: ExposureArg },
    /// Zoom factor 1.0-4.0
    Zoom {
        #[arg(allow_negative_numbers = true)]
        factor: f32,
    },
    /// Move the gimbal to an absolute pose in degrees (switches tracking to static first)
    Gimbal {
        /// Positive turns the camera to its left
        #[arg(long, allow_negative_numbers = true)]
        pan: Option<f32>,
        /// Positive tilts the camera up
        #[arg(long, allow_negative_numbers = true)]
        tilt: Option<f32>,
    },
    /// Field of view (ignored by the camera while it sleeps)
    Fov { fov: FieldOfView },
    /// Status LED brightness 0 (off) to 3
    Led { brightness: u8 },
    /// Mirror the image
    Mirror { state: OnOff },
    /// Invert view and gimbal (reverses the gimbal's pan direction)
    Invert { state: OnOff },
    /// Auto-sleep: "off" or a time in seconds
    AutoSleep { time: String },
    /// Picture shown while the camera sleeps
    SleepBackground { background: SleepBackground },
    /// Mirror the sleep background
    SleepBackgroundMirror { state: OnOff },
    /// Microphone settings
    #[command(subcommand)]
    Audio(Audio),
    /// Gesture control
    #[command(subcommand)]
    Gesture(GestureCmd),
    /// Hand tracking: which hand
    Hand { side: HandSide },
    /// Zone tracking
    #[command(subcommand)]
    Zone(ZoneCmd),
    /// Focus: autofocus mode, continuous autofocus, manual focus
    #[command(subcommand)]
    Focus(FocusCmd),
    /// Initial state: the pose the camera moves to when it powers on (the camera cannot report it)
    #[command(subcommand)]
    Startup(StartupCmd),
    /// UVC image control, e.g. "image brightness 128" (names are in "status")
    Image {
        control: String,
        #[arg(allow_negative_numbers = true)]
        value: i32,
    },
    /// Presets stored in the camera (slots 0-2)
    #[command(subcommand)]
    Preset(PresetCmd),
    /// Reset the camera to factory settings (it restarts and deletes its presets)
    FactoryReset {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ExposureArg {
    Global,
    Face,
    Manual,
}

#[derive(Subcommand)]
pub enum Audio {
    /// Microphone noise reduction
    Noise { level: NoiseReduction },
    /// Automatic gain
    Gain { state: OnOff },
    /// Pickup distance
    Distance { distance: PickupDistance },
    /// Keep the microphone on while the camera sleeps
    SleepMic { state: OnOff },
    /// Microphone on/off; off makes the camera restart (about 18 s)
    Mic { state: OnOff },
}

#[derive(Subcommand)]
pub enum GestureCmd {
    /// Switch one gesture on or off
    Set { gesture: Gesture, state: OnOff },
    /// Zoom ratio of the zoom gestures, 1.0-4.0
    ZoomRatio { ratio: f32 },
}

#[derive(Subcommand)]
pub enum ZoneCmd {
    On,
    Off,
    /// Automatically follow new targets (no readback; remembered in the state file)
    Follow {
        state: OnOff,
    },
    /// Area the zone tracking stays in, in degrees (no readback). Full area: --pan -100 100 --tilt -30 30
    Area {
        /// Pan range, MIN MAX, positive = camera left (-100..100)
        #[arg(long, num_args = 2, value_names = ["MIN", "MAX"], allow_negative_numbers = true, required = true)]
        pan: Vec<f32>,
        /// Tilt range, MIN MAX, positive = up (-30..30)
        #[arg(long, num_args = 2, value_names = ["MIN", "MAX"], allow_negative_numbers = true, required = true)]
        tilt: Vec<f32>,
    },
    /// Take the current camera pose as the position the zone tracking returns to
    ReturnHere,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum FocusMode {
    Face,
    Global,
}

#[derive(Subcommand)]
pub enum FocusCmd {
    /// Autofocus mode
    Mode { mode: FocusMode },
    /// Continuous autofocus on/off
    Auto { state: OnOff },
    /// Manual focus 0-100 (turns autofocus off)
    Set { value: i32 },
}

#[derive(Subcommand)]
pub enum StartupCmd {
    /// Set the start pose: --pan/--tilt/--zoom, or --preset <slot>, or --current. There is
    /// no way known to clear it; pan 0, tilt 0, zoom 1 is the neutral start.
    Set {
        /// Positive turns the camera to its left
        #[arg(long, allow_negative_numbers = true, requires_all = ["tilt", "zoom"])]
        pan: Option<f32>,
        /// Positive tilts the camera up
        #[arg(long, allow_negative_numbers = true, requires_all = ["pan", "zoom"])]
        tilt: Option<f32>,
        #[arg(long, requires_all = ["pan", "tilt"])]
        zoom: Option<f32>,
        /// Use the pose of a stored preset (slot as in "preset list", 0-2)
        #[arg(long, conflicts_with_all = ["pan", "current"])]
        preset: Option<u8>,
        /// Use the current pose (remembered gimbal pose, live zoom)
        #[arg(long, conflicts_with = "pan")]
        current: bool,
    },
}

#[derive(Subcommand)]
pub enum PresetCmd {
    /// List the presets
    List,
    /// Save a pose to a slot; without --pan/--tilt/--zoom the current pose is used
    /// (the last gimbal pose obsbot4linux sent, else the one V4L2 last set; live zoom)
    Save {
        slot: u8,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, allow_negative_numbers = true, requires_all = ["tilt", "zoom"])]
        pan: Option<f32>,
        #[arg(long, allow_negative_numbers = true, requires_all = ["pan", "zoom"])]
        tilt: Option<f32>,
        #[arg(long, requires_all = ["pan", "tilt"])]
        zoom: Option<f32>,
    },
    /// Rename a preset (printable ASCII, up to 32 characters)
    Rename { slot: u8, name: String },
    /// Delete a preset
    Delete { slot: u8 },
    /// Move the camera to a preset
    Recall { slot: u8 },
}

fn onoff(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn value_name<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Runs the command and returns (JSON result, text for humans). Setters answer with
/// {"ok","command","value"}; `status` prints the status object itself.
pub fn execute(cmd: Cmd) -> Result<(Value, String)> {
    // refuse before opening the camera
    if let Cmd::FactoryReset { yes: false } = cmd {
        return Err(Error::Invalid(
            "factory reset deletes presets and settings; pass --yes".into(),
        ));
    }
    if matches!(cmd, Cmd::Daemon | Cmd::Watch) {
        return Err(Error::Invalid(
            "daemon and watch are not camera commands".into(),
        ));
    }
    let cam = Camera::open()?;
    // (command name, value) of a setter
    let done = |name: &str, value: Value| {
        let text = match &value {
            Value::Null => name.to_string(),
            Value::String(s) => format!("{name}: {s}"),
            v => format!("{name}: {v}"),
        };
        Ok((json!({"ok": true, "command": name, "value": value}), text))
    };
    match cmd {
        Cmd::Daemon | Cmd::Watch => unreachable!("handled above"),
        Cmd::Status => {
            let full = value_name(cam.full_status()?);
            let text = flatten(&full);
            Ok((full, text))
        }
        Cmd::Sleep => cam.sleep(true).and_then(|_| done("sleep", Value::Null)),
        Cmd::Wake => cam.sleep(false).and_then(|_| done("wake", Value::Null)),
        Cmd::Tracking { mode } => cam
            .set_ai_mode(mode)
            .and_then(|_| done("tracking", value_name(mode))),
        Cmd::Speed { speed } => cam
            .set_tracking_speed(speed)
            .and_then(|_| done("speed", value_name(speed))),
        Cmd::Hdr { state } => cam
            .set_hdr(state.get())
            .and_then(|_| done("hdr", json!(onoff(state.get())))),
        Cmd::Exposure { mode } => match mode {
            ExposureArg::Manual => cam
                .set_manual_exposure()
                .and_then(|_| done("exposure", json!("manual"))),
            ExposureArg::Global => cam
                .set_exposure_mode(ExposureMode::Global)
                .and_then(|_| done("exposure", json!("global"))),
            ExposureArg::Face => cam
                .set_exposure_mode(ExposureMode::Face)
                .and_then(|_| done("exposure", json!("face"))),
        },
        Cmd::Zoom { factor } => cam
            .set_zoom(factor)
            .and_then(|_| done("zoom", json!(factor))),
        Cmd::Gimbal { pan, tilt } => cam
            .gimbal(pan, tilt)
            .and_then(|_| done("gimbal", json!({"pan": pan, "tilt": tilt}))),
        Cmd::Fov { fov } => cam
            .set_field_of_view(fov)
            .and_then(|_| done("fov", value_name(fov))),
        Cmd::Led { brightness } => cam
            .set_led(brightness)
            .and_then(|_| done("led", json!(brightness))),
        Cmd::Mirror { state } => cam
            .set_mirror(state.get())
            .and_then(|_| done("mirror", json!(onoff(state.get())))),
        Cmd::Invert { state } => cam
            .set_invert(state.get())
            .and_then(|_| done("invert", json!(onoff(state.get())))),
        Cmd::AutoSleep { time } => {
            let seconds = if time == "off" {
                None
            } else {
                Some(
                    time.parse::<u16>()
                        .map_err(|_| Error::Invalid("auto-sleep is \"off\" or seconds".into()))?,
                )
            };
            cam.set_auto_sleep(seconds)
                .and_then(|_| done("auto_sleep", json!(seconds)))
        }
        Cmd::SleepBackground { background } => cam
            .set_sleep_background(background)
            .and_then(|_| done("sleep_background", value_name(background))),
        Cmd::SleepBackgroundMirror { state } => cam
            .set_sleep_background_mirror(state.get())
            .and_then(|_| done("sleep_background_mirror", json!(onoff(state.get())))),
        Cmd::Audio(a) => match a {
            Audio::Noise { level } => cam
                .set_noise_reduction(level)
                .and_then(|_| done("audio_noise", value_name(level))),
            Audio::Gain { state } => cam
                .set_auto_gain(state.get())
                .and_then(|_| done("audio_gain", json!(onoff(state.get())))),
            Audio::Distance { distance } => cam
                .set_pickup_distance(distance)
                .and_then(|_| done("audio_distance", value_name(distance))),
            Audio::SleepMic { state } => cam
                .set_mic_in_sleep(state.get())
                .and_then(|_| done("audio_sleep_mic", json!(onoff(state.get())))),
            Audio::Mic { state } => cam
                .set_microphone(state.get())
                .and_then(|_| done("audio_mic", json!(onoff(state.get())))),
        },
        Cmd::Gesture(g) => match g {
            GestureCmd::Set { gesture, state } => {
                cam.set_gesture(gesture, state.get()).and_then(|_| {
                    done(
                        "gesture",
                        json!({"gesture": gesture, "state": onoff(state.get())}),
                    )
                })
            }
            GestureCmd::ZoomRatio { ratio } => cam
                .set_gesture_zoom_ratio(ratio)
                .and_then(|_| done("gesture_zoom_ratio", json!(ratio))),
        },
        Cmd::Hand { side } => cam
            .set_hand_side(side)
            .and_then(|_| done("hand", value_name(side))),
        Cmd::Zone(z) => match z {
            ZoneCmd::On | ZoneCmd::Off => {
                let on = matches!(z, ZoneCmd::On);
                cam.set_zone_tracking(on)
                    .and_then(|_| done("zone", json!(onoff(on))))
            }
            ZoneCmd::Follow { state } => cam
                .set_zone_follow(state.get())
                .and_then(|_| done("zone_follow", json!(onoff(state.get())))),
            ZoneCmd::Area { pan, tilt } => cam
                .set_zone_area((pan[0], pan[1]), (tilt[0], tilt[1]))
                .and_then(|_| done("zone_area", json!({ "pan": pan, "tilt": tilt }))),
            ZoneCmd::ReturnHere => cam
                .set_zone_return_here()
                .and_then(|_| done("zone_return", json!("here"))),
        },
        Cmd::Focus(f) => match f {
            FocusCmd::Mode { mode } => {
                let face = matches!(mode, FocusMode::Face);
                cam.set_autofocus_mode(face)
                    .and_then(|_| done("focus_mode", json!(if face { "face" } else { "global" })))
            }
            FocusCmd::Auto { state } => cam
                .set_autofocus(state.get())
                .and_then(|_| done("focus_auto", json!(onoff(state.get())))),
            FocusCmd::Set { value } => cam
                .set_focus(value)
                .and_then(|_| done("focus", json!(value))),
        },
        Cmd::Startup(StartupCmd::Set {
            pan,
            tilt,
            zoom,
            preset,
            current,
        }) => {
            let pose = match (pan.zip(tilt).zip(zoom), preset, current) {
                (Some(((pan, tilt), zoom)), None, false) => {
                    cam.set_startup(Pose { pan, tilt, zoom }, None)?;
                    Pose { pan, tilt, zoom }
                }
                (None, Some(slot), false) => cam.set_startup_preset(slot)?,
                (None, None, true) => cam.set_startup_current()?,
                _ => {
                    return Err(Error::Invalid(
                        "give --pan/--tilt/--zoom, or --preset, or --current".into(),
                    ));
                }
            };
            done("startup", json!({"pose": pose, "preset": preset}))
        }
        Cmd::Image { control, value } => cam
            .set_image(&control, value)
            .and_then(|_| done("image", json!({"control": control, "value": value}))),
        Cmd::Preset(p) => match p {
            PresetCmd::List => {
                let list = cam.presets()?;
                let text = if list.is_empty() {
                    "no presets".to_string()
                } else {
                    list.iter()
                        .map(|p| {
                            format!(
                                "slot {}: {:?}  pan {:.2}  tilt {:.2}  zoom {:.2}",
                                p.slot, p.name, p.pose.pan, p.pose.tilt, p.pose.zoom
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                Ok((
                    json!({"ok": true, "command": "preset_list", "value": list}),
                    text,
                ))
            }
            PresetCmd::Save {
                slot,
                name,
                pan,
                tilt,
                zoom,
            } => {
                let pose =
                    pan.zip(tilt)
                        .zip(zoom)
                        .map(|((pan, tilt), zoom)| Pose { pan, tilt, zoom });
                let saved = cam.save_preset(slot, name.as_deref(), pose)?;
                done("preset_save", json!({"slot": slot, "pose": saved}))
            }
            PresetCmd::Rename { slot, name } => cam
                .rename_preset(slot, &name)
                .and_then(|_| done("preset_rename", json!({"slot": slot, "name": name}))),
            PresetCmd::Delete { slot } => cam
                .delete_preset(slot)
                .and_then(|_| done("preset_delete", json!({"slot": slot}))),
            PresetCmd::Recall { slot } => cam
                .recall_preset(slot)
                .and_then(|_| done("preset_recall", json!({"slot": slot}))),
        },
        Cmd::FactoryReset { .. } => cam
            .factory_reset()
            .and_then(|_| done("factory_reset", Value::Null)),
    }
}

/// Text view of nested JSON: one "path: value" line per leaf.
fn flatten(v: &Value) -> String {
    fn walk(prefix: &str, v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => m.iter().for_each(|(k, v)| {
                walk(
                    &if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    },
                    v,
                    out,
                )
            }),
            Value::Array(a) if a.iter().all(Value::is_string) => out.push(format!(
                "{prefix}: {}",
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Value::String(s) => out.push(format!("{prefix}: {s}")),
            v => out.push(format!("{prefix}: {v}")),
        }
    }
    let mut out = Vec::new();
    walk("", v, &mut out);
    out.join("\n")
}

pub fn print_ok(json: bool, result: &Value, text: &str) -> i32 {
    // a closed pipe (`obsbot4linux status | head`) is not an error
    let _ = writeln!(
        std::io::stdout(),
        "{}",
        if json {
            result.to_string()
        } else {
            text.to_string()
        }
    );
    0
}

pub fn print_error(json: bool, message: &str) -> i32 {
    if json {
        let _ = writeln!(
            std::io::stdout(),
            "{}",
            json!({"ok": false, "error": message})
        );
    } else {
        eprintln!("obsbot4linux: {message}");
    }
    1
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Daemon => return report(cli.json, crate::daemon::run()),
        Cmd::Watch => return report(cli.json, crate::client::watch()),
        _ => {}
    }
    if !cli.direct {
        let args = std::env::args()
            .skip(1)
            .filter(|a| a != "--direct")
            .collect();
        if let Some(code) = crate::client::forward(&cli, args) {
            return code;
        }
    }
    match execute(cli.command) {
        Ok((json, text)) => print_ok(cli.json, &json, &text),
        Err(e) => print_error(cli.json, &e.to_string()),
    }
}

fn report(json: bool, result: Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => print_error(json, &e.to_string()),
    }
}
