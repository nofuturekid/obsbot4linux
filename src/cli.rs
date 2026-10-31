// SPDX-License-Identifier: GPL-3.0-or-later

//! Argument parsing and output. Every command prints text, or JSON with `--json`.

use std::io::Write;

use clap::{Parser, Subcommand, ValueEnum};
use obsbot4linux::camera::{Camera, Identity};
use obsbot4linux::commands::*;
use obsbot4linux::device::Detected;
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

#[derive(Subcommand)]
pub enum HandCmd {
    /// Track the right hand
    Right,
    /// Track the left hand
    Left,
    /// Take the current camera pose as the position the camera returns to after the hand leaves
    ReturnHere,
    /// Move the camera to the hand-tracking return position
    ReturnGo,
    /// Clear the hand-tracking return position
    ReturnReset,
}

#[derive(Subcommand)]
pub enum LedCmd {
    /// Let the status LED blink (on) or burn steadily (off). The camera does not report it
    Blink { state: OnOff },
}

#[derive(Subcommand)]
pub enum WakeCmd {
    /// Zoom factor 1.0-4.0 the camera takes after waking up (the camera stores 0-100 percent)
    Zoom { factor: f32 },
    /// Tracking mode the camera starts in (the current mode stays)
    Mode { mode: AiMode },
}

#[derive(Subcommand)]
pub enum GimbalCmd {
    /// Stop all gimbal axes (also a `gimbal rate` still running)
    Stop,
    /// Turn the gimbal so that a point of the picture moves to its centre: x and y from 0.0 to
    /// 1.0, from the top left (switches tracking to static first)
    #[command(hide = true)]
    Center { x: f32, y: f32 },
    /// Keep turning at a speed in degrees per second (up to 178) UNTIL `gimbal stop` or a speed
    /// of 0, also when the client is gone; like the joystick of OBSBOT Center (switches
    /// tracking to static first)
    Rate {
        /// Positive turns the camera to its left
        #[arg(long, allow_negative_numbers = true)]
        pan: Option<f32>,
        /// Positive tilts the camera up
        #[arg(long, allow_negative_numbers = true)]
        tilt: Option<f32>,
    },
}

#[derive(Subcommand)]
pub enum ZoomCmd {
    /// Stop a zoom in progress
    Stop,
}

#[derive(Subcommand)]
pub enum FramingCmd {
    /// Auto zoom of the human tracking on or off
    AutoZoom { state: OnOff },
    /// Size of the auto zoom framing
    Size { size: FramingSize },
    /// How fast the auto zoom follows
    ZoomSpeed { speed: AutoZoomSpeed },
    /// Shift the framing by a fraction of the frame, -0.16 to 0.16
    Offset {
        #[arg(long, allow_negative_numbers = true)]
        x: Option<f32>,
        #[arg(long, allow_negative_numbers = true)]
        y: Option<f32>,
    },
    /// Which part of the body the framing follows
    Part { part: PartMode },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl OnOff {
    pub fn get(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the daemon: owns the camera, serves the socket, broadcasts state changes
    Daemon,
    /// Stream state events as JSON lines and forward JSON requests from stdin (for widgets)
    Watch,
    /// Ask the daemon to poll the gimbal pose for this connection (a `watch` request; the daemon
    /// polls it every second only while a connection wants it). Not usable as a one-off call
    LivePose { state: OnOff },
    /// Exit 0 and print the model (--json: model and video device) when a supported camera is
    /// connected, exit 1 with no output when not. Enumeration only: no daemon, no camera command
    Detect,
    /// Show all settings (the camera's status block, tracking readback, zoom, image controls, remembered state)
    Status,
    /// Put the camera to sleep
    Sleep,
    /// Wake the camera, or set what it does on waking: `wake zoom`, `wake mode`
    Wake {
        #[command(subcommand)]
        setting: Option<WakeCmd>,
    },
    /// AI tracking mode
    Tracking { mode: AiMode },
    /// Tracking type: standard, sport (fixed fast gain) or headroom (more room above the head)
    Speed { speed: TrackingSpeed },
    /// Fine tracking speed, from lazy to very-fast, or auto. Ignored while `speed sport` is set
    /// or the part mode is not normal
    SpeedMode { mode: SpeedMode },
    /// Framing options of the human tracking; an AI mode change overwrites them
    #[command(subcommand)]
    Framing(FramingCmd),
    /// HDR
    Hdr { state: OnOff },
    /// Exposure: automatic metering over the whole picture (global) or on faces (face), or manual,
    /// optionally with the shutter time and gain (`image gain` sets the gain too; `image exposure_time`
    /// is refused unless the exposure is already manual)
    Exposure {
        mode: ExposureArg,
        /// Manual only: shutter time in 100 us units, 1-2500 (330 is one frame at 30 fps); the
        /// camera uses the step at or below it
        #[arg(long)]
        time: Option<i32>,
        /// Manual only: gain 1-64 (ISO is gain x 100)
        #[arg(long)]
        gain: Option<i32>,
    },
    /// Zoom factor 1.0-4.0; with --speed the camera zooms smoothly instead of jumping
    #[command(args_conflicts_with_subcommands = true)]
    Zoom {
        #[arg(allow_negative_numbers = true)]
        factor: Option<f32>,
        /// Zoom speed (OBSBOT Center uses 30; the auto zoom 10, 30, 50)
        #[arg(long, requires = "factor")]
        speed: Option<i32>,
        #[command(subcommand)]
        stop: Option<ZoomCmd>,
    },
    /// Move the gimbal to an absolute pose in degrees (switches tracking to static first), or
    /// `gimbal stop|rate`
    #[command(args_conflicts_with_subcommands = true)]
    Gimbal {
        /// Positive turns the camera to its left
        #[arg(long, allow_negative_numbers = true)]
        pan: Option<f32>,
        /// Positive tilts the camera up
        #[arg(long, allow_negative_numbers = true)]
        tilt: Option<f32>,
        /// Take as long as this many degrees per second (1-180) need instead of going as fast
        /// as possible
        #[arg(long)]
        speed: Option<f32>,
        #[command(subcommand)]
        action: Option<GimbalCmd>,
    },
    /// Field of view (ignored by the camera while it sleeps)
    Fov { fov: FieldOfView },
    /// Status LED: brightness 0 (off) to 3, or `led blink on|off`
    #[command(args_conflicts_with_subcommands = true)]
    Led {
        brightness: Option<u8>,
        #[command(subcommand)]
        blink: Option<LedCmd>,
    },
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
    /// Hand tracking: which hand, return position
    #[command(subcommand)]
    Hand(HandCmd),
    /// Zone tracking
    #[command(subcommand)]
    Zone(ZoneCmd),
    /// Focus: autofocus mode, continuous autofocus, manual focus
    #[command(subcommand)]
    Focus(FocusCmd),
    /// Initial state: the pose the camera moves to when it powers on
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
    /// Microphone volume 0-100 (below 50 reads as near, 50-84 standard, from 85 far)
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        volume: u8,
    },
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
    /// Automatically follow new targets
    Follow {
        state: OnOff,
    },
    /// Area the zone tracking stays in, in degrees. Full area: --pan -150 150 --tilt -70 90
    Area {
        /// Pan range, MIN MAX, positive = camera left (-150..150)
        #[arg(long, num_args = 2, value_names = ["MIN", "MAX"], allow_negative_numbers = true, required = true)]
        pan: Vec<f32>,
        /// Tilt range, MIN MAX, positive = up (-70..90)
        #[arg(long, num_args = 2, value_names = ["MIN", "MAX"], allow_negative_numbers = true, required = true)]
        tilt: Vec<f32>,
    },
    /// Take the current camera pose as the position the zone tracking returns to
    ReturnHere,
    /// Move the camera to the zone return position
    ReturnGo,
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
    /// The camera's own autofocus mode: continuous, single (focus once and hold) or manual
    Af { mode: AutofocusMode },
    /// Focus once on a point: x and y from 0.0 to 1.0, from the top left of the picture
    Point { x: f32, y: f32 },
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
    /// Show the start pose and the picture settings of the initial state
    Show,
    /// Move the camera to the start pose now (the pose it takes at power-on)
    Go,
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
    execute_with(cmd, None)
}

/// Like [`execute`]; the daemon passes the identity it already read, so `status` does not ask
/// the camera for the firmware again.
pub fn execute_with(cmd: Cmd, identity: Option<Identity>) -> Result<(Value, String)> {
    // refuse before opening the camera
    if let Cmd::FactoryReset { yes: false } = cmd {
        return Err(Error::Invalid(
            "factory reset deletes presets and settings; pass --yes".into(),
        ));
    }
    if matches!(cmd, Cmd::Daemon | Cmd::Watch | Cmd::Detect) {
        return Err(Error::Invalid(
            "daemon, watch and detect are not camera commands".into(),
        ));
    }
    if matches!(cmd, Cmd::LivePose { .. }) {
        return Err(Error::Invalid(LIVE_POSE_ONE_OFF.into()));
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
        Cmd::Daemon | Cmd::Watch | Cmd::Detect | Cmd::LivePose { .. } => {
            unreachable!("handled above")
        }
        Cmd::Status => {
            let full = value_name(cam.full_status_with(None, identity)?);
            // the box is for machines: plain text only says whether the camera sees its target
            let mut plain = full.clone();
            if let Some(m) = plain.as_object_mut() {
                m.remove("target");
            }
            let mut text = flatten(&plain);
            match full["target"]["detected"].as_bool() {
                Some(true) => text.push_str("\ntarget: detected"),
                Some(false) => text.push_str("\ntarget: searching"),
                None => {}
            }
            Ok((full, text))
        }
        Cmd::Sleep => cam.sleep(true).and_then(|_| done("sleep", Value::Null)),
        Cmd::Wake { setting: None } => cam.sleep(false).and_then(|_| done("wake", Value::Null)),
        Cmd::Wake {
            setting: Some(WakeCmd::Zoom { factor }),
        } => cam
            .set_wake_zoom(factor)
            .and_then(|percent| done("wake_zoom", json!({"factor": factor, "percent": percent}))),
        Cmd::Wake {
            setting: Some(WakeCmd::Mode { mode }),
        } => cam
            .set_default_work_mode(mode)
            .and_then(|_| done("wake_mode", value_name(mode))),
        Cmd::Tracking { mode } => cam
            .set_ai_mode(mode)
            .and_then(|_| done("tracking", value_name(mode))),
        Cmd::Speed { speed } => cam
            .set_tracking_speed(speed)
            .and_then(|_| done("speed", value_name(speed))),
        Cmd::SpeedMode { mode } => cam
            .set_speed_mode(mode)
            .and_then(|_| done("speed_mode", value_name(mode))),
        Cmd::Framing(f) => match f {
            FramingCmd::AutoZoom { state } => cam
                .set_auto_zoom(state.get())
                .and_then(|_| done("framing_auto_zoom", json!(onoff(state.get())))),
            FramingCmd::Size { size } => cam
                .set_framing_size(size)
                .and_then(|_| done("framing_size", value_name(size))),
            FramingCmd::ZoomSpeed { speed } => cam
                .set_auto_zoom_speed(speed)
                .and_then(|_| done("framing_zoom_speed", value_name(speed))),
            FramingCmd::Offset { x, y } => cam
                .set_framing_offset(x, y)
                .and_then(|_| done("framing_offset", json!({"x": x, "y": y}))),
            FramingCmd::Part { part } => cam
                .set_part_mode(part)
                .and_then(|_| done("framing_part", value_name(part))),
        },
        Cmd::Hdr { state } => cam
            .set_hdr(state.get())
            .and_then(|_| done("hdr", json!(onoff(state.get())))),
        Cmd::Exposure { mode, time, gain } => match mode {
            ExposureArg::Manual => cam.set_manual_exposure(time, gain).and_then(|_| {
                done(
                    "exposure",
                    if time.is_none() && gain.is_none() {
                        json!("manual")
                    } else {
                        json!({"mode": "manual", "time": time, "gain": gain})
                    },
                )
            }),
            _ if time.is_some() || gain.is_some() => Err(Error::Invalid(
                "--time and --gain only go with manual exposure".into(),
            )),
            ExposureArg::Global => cam
                .set_exposure_mode(ExposureMode::Global)
                .and_then(|_| done("exposure", json!("global"))),
            ExposureArg::Face => cam
                .set_exposure_mode(ExposureMode::Face)
                .and_then(|_| done("exposure", json!("face"))),
        },
        Cmd::Zoom {
            stop: Some(ZoomCmd::Stop),
            ..
        } => cam.zoom_stop().and_then(|_| done("zoom_stop", Value::Null)),
        Cmd::Zoom {
            factor: Some(factor),
            speed: Some(speed),
            ..
        } => cam
            .zoom_to(factor, speed)
            .and_then(|_| done("zoom", json!({"factor": factor, "speed": speed}))),
        Cmd::Zoom {
            factor: Some(factor),
            ..
        } => cam
            .set_zoom(factor)
            .and_then(|_| done("zoom", json!(factor))),
        Cmd::Zoom { .. } => Err(Error::Invalid(
            "give a zoom factor (with --speed to zoom smoothly), or `stop`".into(),
        )),
        Cmd::Gimbal {
            action: Some(action),
            ..
        } => match action {
            GimbalCmd::Stop => cam
                .gimbal_stop()
                .and_then(|_| done("gimbal_stop", Value::Null)),
            GimbalCmd::Center { x, y } => cam
                .gimbal_center(x, y)
                .and_then(|_| done("gimbal_center", json!({"x": x, "y": y}))),
            GimbalCmd::Rate { pan, tilt } => {
                if pan.is_none() && tilt.is_none() {
                    return Err(Error::Invalid("give --pan and/or --tilt".into()));
                }
                cam.gimbal_rate(pan.unwrap_or(0.0), tilt.unwrap_or(0.0))
                    .and_then(|_| done("gimbal_rate", json!({"pan": pan, "tilt": tilt})))
            }
        },
        Cmd::Gimbal {
            pan, tilt, speed, ..
        } => cam
            .gimbal(pan, tilt, speed)
            .and_then(|_| done("gimbal", json!({"pan": pan, "tilt": tilt, "speed": speed}))),
        Cmd::Fov { fov } => cam
            .set_field_of_view(fov)
            .and_then(|_| done("fov", value_name(fov))),
        Cmd::Led {
            blink: Some(LedCmd::Blink { state }),
            ..
        } => cam
            .set_led_blink(state.get())
            .and_then(|_| done("led_blink", json!(onoff(state.get())))),
        Cmd::Led {
            brightness: Some(brightness),
            ..
        } => cam
            .set_led(brightness)
            .and_then(|_| done("led", json!(brightness))),
        Cmd::Led { .. } => Err(Error::Invalid(
            "give a brightness 0-3, or `blink on|off`".into(),
        )),
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
            Audio::Volume { volume } => cam
                .set_audio_volume(volume)
                .and_then(|_| done("audio_volume", json!(volume))),
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
        Cmd::Hand(h) => match h {
            HandCmd::Right | HandCmd::Left => {
                let side = if matches!(h, HandCmd::Left) {
                    HandSide::Left
                } else {
                    HandSide::Right
                };
                cam.set_hand_side(side)
                    .and_then(|_| done("hand", value_name(side)))
            }
            HandCmd::ReturnHere => cam
                .set_hand_return_here()
                .and_then(|pose| done("hand_return", json!({"pose": pose}))),
            HandCmd::ReturnGo => cam
                .hand_return_go()
                .and_then(|_| done("hand_return_go", Value::Null)),
            HandCmd::ReturnReset => cam
                .hand_return_reset()
                .and_then(|_| done("hand_return_reset", Value::Null)),
        },
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
                .and_then(|pose| done("zone_return", json!({"pose": pose}))),
            ZoneCmd::ReturnGo => cam
                .zone_return_go()
                .and_then(|_| done("zone_return_go", Value::Null)),
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
            FocusCmd::Af { mode } => cam
                .set_autofocus_select(mode)
                .and_then(|_| done("focus_af", value_name(mode))),
            FocusCmd::Point { x, y } => cam
                .focus_point(x, y)
                .and_then(|_| done("focus_point", json!({"x": x, "y": y}))),
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
        Cmd::Startup(StartupCmd::Show) => {
            // the pose is the point of the command: a refused actions query leaves them null
            let pose = cam.startup()?;
            let actions = cam.startup_actions().ok();
            let value = json!({"pose": pose, "actions": actions});
            let text = flatten(&value);
            Ok((
                json!({"ok": true, "command": "startup_show", "value": value}),
                text,
            ))
        }
        Cmd::Startup(StartupCmd::Go) => cam
            .startup_go()
            .and_then(|_| done("startup_go", Value::Null)),
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

const LIVE_POSE_ONE_OFF: &str = "live-pose applies to one connection of the daemon: send it as a request over `obsbot4linux watch` (it ends when that connection closes), not as a one-off call";

pub fn run() -> i32 {
    let cli = Cli::parse();
    match cli.command {
        // the daemon would accept it and forget it with this process's connection
        Cmd::LivePose { .. } => return print_error(cli.json, LIVE_POSE_ONE_OFF),
        Cmd::Daemon => return report(cli.json, crate::daemon::run()),
        Cmd::Watch => return report(cli.json, crate::client::watch()),
        Cmd::Detect => return detect(cli.json),
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

/// `obsbot4linux detect`: exit 0 and the model (and device) of a supported camera, exit 1 and
/// no output without one. Never goes through the daemon.
fn detect(json: bool) -> i32 {
    let Some(found) = obsbot4linux::device::detect() else {
        return 1;
    };
    let (value, text) = detect_output(&found);
    print_ok(json, &value, &text)
}

fn detect_output(found: &Detected) -> (Value, String) {
    let device = found.device.to_string_lossy();
    (
        json!({"model": found.model.name, "device": device}),
        found.model.name.to_string(),
    )
}

fn report(json: bool, result: Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => print_error(json, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsbot4linux::models::MODELS;

    fn parse(args: &str) -> std::result::Result<Cmd, clap::Error> {
        Cli::try_parse_from(std::iter::once("obsbot4linux").chain(args.split(' ')))
            .map(|c| c.command)
    }

    #[test]
    fn led_keeps_its_plain_brightness_and_gains_blink() {
        // `led 2` is what clients send today; it must not turn into a subcommand error
        assert!(matches!(
            parse("led 2"),
            Ok(Cmd::Led {
                brightness: Some(2),
                blink: None
            })
        ));
        assert!(matches!(
            parse("led blink on"),
            Ok(Cmd::Led {
                brightness: None,
                blink: Some(LedCmd::Blink { state: OnOff::On })
            })
        ));
        assert!(parse("led 2 blink on").is_err());
    }

    #[test]
    fn wake_without_arguments_still_wakes() {
        assert!(matches!(parse("wake"), Ok(Cmd::Wake { setting: None })));
        assert!(matches!(
            parse("wake zoom 2.5"),
            Ok(Cmd::Wake {
                setting: Some(WakeCmd::Zoom { .. })
            })
        ));
        assert!(matches!(
            parse("wake mode upper-body"),
            Ok(Cmd::Wake {
                setting: Some(WakeCmd::Mode {
                    mode: AiMode::UpperBody
                })
            })
        ));
    }

    #[test]
    fn exposure_manual_takes_time_and_gain_and_volume_is_bounded() {
        assert!(matches!(
            parse("exposure manual --time 100 --gain 7"),
            Ok(Cmd::Exposure {
                time: Some(100),
                gain: Some(7),
                ..
            })
        ));
        assert!(matches!(
            parse("exposure manual"),
            Ok(Cmd::Exposure {
                time: None,
                gain: None,
                ..
            })
        ));
        assert!(parse("audio volume 100").is_ok());
        assert!(parse("audio volume 101").is_err());
        assert!(parse("audio volume -1").is_err());
        assert!(matches!(
            parse("focus af single"),
            Ok(Cmd::Focus(FocusCmd::Af {
                mode: AutofocusMode::Single
            }))
        ));
        assert!(parse("focus point 0.3 0.4").is_ok());
    }

    #[test]
    fn gimbal_and_zoom_keep_their_syntax_and_gain_subcommands() {
        // what the widget and scripts send today
        assert!(matches!(
            parse("gimbal --pan 10 --tilt -5"),
            Ok(Cmd::Gimbal {
                pan: Some(_),
                tilt: Some(_),
                speed: None,
                action: None
            })
        ));
        assert!(matches!(
            parse("zoom 2.5"),
            Ok(Cmd::Zoom {
                factor: Some(_),
                speed: None,
                stop: None
            })
        ));
        assert!(matches!(
            parse("speed sport"),
            Ok(Cmd::Speed {
                speed: TrackingSpeed::Sport
            })
        ));
        assert!(matches!(
            parse("gimbal --pan 10 --speed 20"),
            Ok(Cmd::Gimbal { speed: Some(_), .. })
        ));
        assert!(matches!(
            parse("gimbal stop"),
            Ok(Cmd::Gimbal {
                action: Some(GimbalCmd::Stop),
                ..
            })
        ));
        assert!(matches!(
            parse("gimbal center 0.3 0.4"),
            Ok(Cmd::Gimbal {
                action: Some(GimbalCmd::Center { .. }),
                ..
            })
        ));
        assert!(matches!(
            parse("gimbal rate --pan -5 --tilt 3"),
            Ok(Cmd::Gimbal {
                action: Some(GimbalCmd::Rate {
                    pan: Some(_),
                    tilt: Some(_)
                }),
                ..
            })
        ));
        assert!(matches!(
            parse("zoom 3 --speed 30"),
            Ok(Cmd::Zoom {
                speed: Some(30),
                ..
            })
        ));
        assert!(matches!(
            parse("zoom stop"),
            Ok(Cmd::Zoom {
                stop: Some(ZoomCmd::Stop),
                ..
            })
        ));
        assert!(
            parse("zoom --speed 30").is_err(),
            "a speed without a factor"
        );
        // a pose and a subcommand are two different requests
        assert!(parse("gimbal --pan 10 stop").is_err());
        assert!(matches!(
            parse("speed headroom"),
            Ok(Cmd::Speed {
                speed: TrackingSpeed::Headroom
            })
        ));
    }

    #[test]
    fn framing_commands_parse() {
        assert!(matches!(
            parse("speed-mode very-fast"),
            Ok(Cmd::SpeedMode {
                mode: SpeedMode::VeryFast
            })
        ));
        assert!(parse("speed-mode turbo").is_err());
        assert!(matches!(
            parse("framing size medium-close-up"),
            Ok(Cmd::Framing(FramingCmd::Size {
                size: FramingSize::MediumCloseUp
            }))
        ));
        assert!(matches!(
            parse("framing offset --x -0.1 --y 0.05"),
            Ok(Cmd::Framing(FramingCmd::Offset {
                x: Some(_),
                y: Some(_)
            }))
        ));
        assert!(parse("framing auto-zoom on").is_ok());
        assert!(parse("framing zoom-speed slow").is_ok());
        assert!(parse("framing part lower-body").is_ok());
        assert!(parse("framing part feet").is_err());
    }

    #[test]
    fn detect_prints_the_model_and_the_streaming_node() {
        let found = Detected {
            model: &MODELS[0],
            device: "/dev/v4l/by-id/usb-X-video-index0".into(),
        };
        let (value, text) = detect_output(&found);
        assert_eq!(text, "Tiny 2 Lite");
        assert_eq!(
            value,
            json!({"model": "Tiny 2 Lite", "device": "/dev/v4l/by-id/usb-X-video-index0"})
        );
    }
}
