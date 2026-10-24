// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! High-level camera operations (one device open per operation).

use crate::commands::{self as cmd, *};
use crate::device::{CtrlRange, Device};
use crate::error::{Error, Result};
use crate::frame::{Answer, Sequence, parse_answer, query_frame};
use crate::presets::{self, Pose, Preset};
use crate::state::{Startup, State};
use crate::status::{Status, Tracking};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::Duration;

const REPLY_GAP: Duration = Duration::from_millis(50);
const REPLY_ATTEMPTS: usize = 20;
/// Gap between the two frames of a pair (zone tracking, preset delete): back to back the
/// camera applies neither.
const PAIR_GAP: Duration = Duration::from_millis(100);

/// Commands t2l supports, for the `capabilities` list of `status --json`.
pub const CAPABILITIES: &[&str] = &[
    "sleep",
    "wake",
    "tracking",
    "speed",
    "hdr",
    "exposure",
    "zoom",
    "gimbal",
    "fov",
    "led",
    "mirror",
    "invert",
    "auto_sleep",
    "sleep_background",
    "sleep_background_mirror",
    "audio",
    "gesture",
    "hand",
    "zone",
    "zone_follow",
    "focus",
    "startup",
    "image",
    "preset",
    "factory_reset",
];

/// UVC controls for the picture, by CLI name (V4L2 control ids).
pub const IMAGE_CONTROLS: &[(&str, u32)] = &[
    ("brightness", 0x0098_0900),
    ("contrast", 0x0098_0901),
    ("saturation", 0x0098_0902),
    ("hue", 0x0098_0903),
    ("white_balance_auto", 0x0098_090c),
    ("gamma", 0x0098_0910),
    ("gain", 0x0098_0913),
    ("power_line_frequency", 0x0098_0918),
    ("white_balance_temperature", 0x0098_091a),
    ("sharpness", 0x0098_091b),
    ("backlight_compensation", 0x0098_091c),
    ("auto_exposure", 0x009a_0901),
    ("exposure_time", 0x009a_0902),
    ("focus", 0x009a_090a),
    ("autofocus", 0x009a_090c),
];
const FOCUS_ABSOLUTE: u32 = 0x009a_090a;
/// V4L2 auto_exposure (exposure_auto): 0 automatic, 1 manual.
const EXPOSURE_AUTO: u32 = 0x009a_0901;
const FOCUS_AUTO: u32 = 0x009a_090c;
const ZOOM_ABSOLUTE: u32 = 0x009a_090d;
const PAN_ABSOLUTE: u32 = 0x009a_0908;
const TILT_ABSOLUTE: u32 = 0x009a_0909;
const MAX_ZOOM: f32 = 4.0;

#[derive(Debug, Clone, Serialize)]
pub struct ImageControl {
    pub value: i32,
    #[serde(flatten)]
    pub range: CtrlRange,
}

#[derive(Debug, Serialize)]
pub struct Zoom {
    /// Current zoom factor from query 0x6804 (follows gestures and preset recalls).
    pub factor: Option<f32>,
    /// UVC zoom_absolute, 0-100 = 1x-4x: the value last written over V4L2. It does not follow
    /// a field of view change, a gesture or a preset recall; `factor` does.
    pub absolute: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct FullStatus {
    /// Always true here; the daemon reports false when the camera is gone.
    pub available: bool,
    pub status: Status,
    /// None when the camera did not answer query 0x0104.
    pub tracking: Option<Tracking>,
    pub zoom: Zoom,
    pub image: BTreeMap<String, ImageControl>,
    pub state: State,
    pub capabilities: Vec<&'static str>,
}

/// One sequence for the whole process, so frames from separate `Camera` handles (the daemon
/// opens the device per operation) never repeat a number.
fn sequence() -> &'static Sequence {
    static SEQ: OnceLock<Sequence> = OnceLock::new();
    SEQ.get_or_init(Sequence::new)
}

pub struct Camera {
    dev: Device,
    seq: &'static Sequence,
}

impl Camera {
    pub fn open() -> Result<Self> {
        Ok(Self {
            dev: Device::open()?,
            seq: sequence(),
        })
    }

    fn short(&self, payload: &[u8]) -> Result<()> {
        self.dev.xu_set(6, payload)
    }

    fn framed(&self, frame: &[u8]) -> Result<()> {
        self.dev.xu_set(2, frame)
    }

    fn pair(&self, frames: [[u8; 60]; 2]) -> Result<()> {
        self.framed(&frames[0])?;
        sleep(PAIR_GAP);
        self.framed(&frames[1])
    }

    /// Sends a query and polls selector 2 for the answer with the same sequence number.
    fn query(&self, command: u16) -> Result<Vec<u8>> {
        let seq = self.seq.next();
        self.framed(&query_frame(0x04, command, seq))?;
        for _ in 0..REPLY_ATTEMPTS {
            sleep(REPLY_GAP);
            // a read before the answer is ready can fail; just poll again
            if let Ok(reply) = self.dev.xu_get(2) {
                match parse_answer(&reply, seq) {
                    Some(Answer::Value(data)) => return Ok(data.to_vec()),
                    Some(Answer::Rejected(code)) => return Err(Error::Rejected(command, code)),
                    None => {}
                }
            }
        }
        Err(Error::NoAnswer(command))
    }

    // --- reading ---

    pub fn status(&self) -> Result<Status> {
        Ok(Status::decode(&self.dev.xu_get(6)?))
    }

    /// Query 0x0104.
    pub fn tracking(&self) -> Result<Tracking> {
        Tracking::decode(&self.query(0x0104)?).ok_or(Error::NoAnswer(0x0104))
    }

    /// Query 0x6804: current zoom factor.
    pub fn zoom_factor(&self) -> Result<f32> {
        let d = self.query(0x6804)?;
        let bytes: [u8; 4] = d
            .get(..4)
            .and_then(|s| s.try_into().ok())
            .ok_or(Error::NoAnswer(0x6804))?;
        Ok(f32::from_le_bytes(bytes))
    }

    pub fn image_controls(&self) -> Result<BTreeMap<String, ImageControl>> {
        let mut map = BTreeMap::new();
        for (name, id) in IMAGE_CONTROLS {
            if let Some(range) = self.dev.ctrl_range(*id)? {
                map.insert(
                    name.to_string(),
                    ImageControl {
                        value: self.dev.get_ctrl(*id)?,
                        range,
                    },
                );
            }
        }
        Ok(map)
    }

    pub fn full_status(&self) -> Result<FullStatus> {
        let status = self.status()?;
        let tracking = match self.tracking() {
            Ok(t) => Some(t),
            Err(Error::NoAnswer(_) | Error::Rejected(..)) => None,
            Err(e) => return Err(e),
        };
        let factor = self.zoom_factor().ok();
        let absolute = self.dev.get_ctrl(ZOOM_ABSOLUTE).ok();
        Ok(FullStatus {
            available: true,
            status,
            tracking,
            zoom: Zoom { factor, absolute },
            image: self.image_controls()?,
            state: State::load(),
            capabilities: CAPABILITIES.to_vec(),
        })
    }

    // --- simple settings ---

    pub fn sleep(&self, asleep: bool) -> Result<()> {
        self.framed(&cmd::sleep(asleep, self.seq.next()))
    }
    pub fn set_ai_mode(&self, mode: AiMode) -> Result<()> {
        self.short(&cmd::ai_mode(mode))?;
        if mode != AiMode::Static {
            // tracking moves the gimbal, so the remembered pose is no longer true
            State {
                gimbal: None,
                ..State::load()
            }
            .save()?;
        }
        Ok(())
    }
    pub fn set_tracking_speed(&self, speed: TrackingSpeed) -> Result<()> {
        self.framed(&cmd::tracking_speed(speed, self.seq.next()))
    }
    pub fn set_hdr(&self, on: bool) -> Result<()> {
        self.short(&cmd::hdr(on))
    }
    pub fn set_auto_sleep(&self, seconds: Option<u16>) -> Result<()> {
        // off = the current time negated, as OBSBOT Center does
        let secs = match seconds {
            Some(s) => i16::try_from(s)
                .map_err(|_| Error::Invalid("auto-sleep time is at most 32767 s".into()))?,
            None => -(self.status()?.auto_sleep_seconds as i16),
        };
        self.short(&cmd::auto_sleep(secs))
    }
    pub fn set_sleep_background(&self, b: SleepBackground) -> Result<()> {
        self.short(&cmd::sleep_background(b))
    }
    pub fn set_sleep_background_mirror(&self, on: bool) -> Result<()> {
        self.short(&cmd::sleep_background_mirror(on))
    }
    pub fn set_led(&self, brightness: u8) -> Result<()> {
        self.short(&cmd::led(brightness)?)
    }
    pub fn set_field_of_view(&self, fov: FieldOfView) -> Result<()> {
        self.short(&cmd::field_of_view(fov))
    }
    pub fn set_mirror(&self, on: bool) -> Result<()> {
        self.short(&cmd::mirror(on))
    }
    pub fn set_invert(&self, on: bool) -> Result<()> {
        self.framed(&cmd::invert(on, self.seq.next()))
    }
    pub fn set_mic_in_sleep(&self, on: bool) -> Result<()> {
        self.short(&cmd::mic_in_sleep(on))
    }
    pub fn set_noise_reduction(&self, level: NoiseReduction) -> Result<()> {
        self.short(&cmd::noise_reduction(level))
    }
    pub fn set_auto_gain(&self, on: bool) -> Result<()> {
        self.short(&cmd::auto_gain(on))
    }
    pub fn set_pickup_distance(&self, d: PickupDistance) -> Result<()> {
        self.short(&cmd::pickup_distance(d))
    }
    pub fn set_microphone(&self, on: bool) -> Result<()> {
        self.short(&cmd::microphone(on))
    }
    pub fn set_gesture(&self, g: Gesture, on: bool) -> Result<()> {
        self.framed(&cmd::gesture(g, on, self.seq.next()))
    }
    pub fn set_gesture_zoom_ratio(&self, ratio: f32) -> Result<()> {
        self.framed(&cmd::gesture_zoom_ratio(ratio, self.seq.next())?)
    }
    pub fn set_hand_side(&self, side: HandSide) -> Result<()> {
        self.framed(&cmd::hand_side(side, self.seq.next()))
    }
    pub fn set_zone_tracking(&self, on: bool) -> Result<()> {
        self.pair(cmd::zone_tracking(on, self.seq.next(), self.seq.next()))
    }

    pub fn set_zone_follow(&self, on: bool) -> Result<()> {
        self.framed(&cmd::zone_follow(on, self.seq.next()))?;
        State {
            zone_follow: Some(on),
            ..State::load()
        }
        .save()
    }

    /// Autofocus mode: face or global (status byte 0x0d follows).
    pub fn set_autofocus_mode(&self, face: bool) -> Result<()> {
        self.framed(&cmd::autofocus_mode(face, self.seq.next()))
    }

    /// Continuous autofocus on/off (UVC focus_automatic_continuous; status byte 0x0e follows).
    pub fn set_autofocus(&self, on: bool) -> Result<()> {
        self.dev.set_ctrl(FOCUS_AUTO, on as i32)
    }

    /// Manual focus 0-100 (UVC focus_absolute; status byte 0x0f follows); turns autofocus off.
    pub fn set_focus(&self, value: i32) -> Result<()> {
        let r = self
            .dev
            .ctrl_range(FOCUS_ABSOLUTE)?
            .ok_or_else(|| Error::Unsupported("no focus control".into()))?;
        if value < r.min || value > r.max {
            return Err(Error::Invalid(format!("focus is {}-{}", r.min, r.max)));
        }
        self.dev.set_ctrl(FOCUS_AUTO, 0)?;
        self.dev.set_ctrl(FOCUS_ABSOLUTE, value)
    }

    /// Initial state: the pose the camera moves to at power-on, then "no picture settings".
    /// There is no readback, so it goes into the state file.
    pub fn set_startup(&self, pose: Pose, preset: Option<u8>) -> Result<()> {
        let source = preset.map_or(3, u32::from);
        let pose_frame = presets::startup_pose(source, pose, self.seq.next())?;
        self.framed(&pose_frame)?;
        sleep(PAIR_GAP);
        self.framed(&presets::startup_picture_settings(self.seq.next()))?;
        let startup = Startup {
            preset,
            pan: pose.pan,
            tilt: pose.tilt,
            zoom: pose.zoom,
        };
        State {
            startup: Some(startup),
            ..State::load()
        }
        .save()
    }

    /// Start pose from a stored preset.
    pub fn set_startup_preset(&self, slot: u8) -> Result<Pose> {
        presets::check_slot(slot)?;
        let p = self.presets()?.into_iter().find(|p| p.slot == slot);
        let p = p.ok_or_else(|| Error::Invalid(format!("preset slot {slot} is empty")))?;
        self.set_startup(p.pose, Some(slot))?;
        Ok(p.pose)
    }

    /// Start pose from the current pose (remembered gimbal pose, live zoom).
    pub fn set_startup_current(&self) -> Result<Pose> {
        let pose = self.current_pose()?;
        self.set_startup(pose, None)?;
        Ok(pose)
    }

    /// Automatic exposure mode: the "auto on" frame, then the face/global selection. The camera does
    /// not report it, so it goes into the state file.
    pub fn set_exposure_mode(&self, mode: ExposureMode) -> Result<()> {
        // back to automatic exposure in case it was set to manual
        self.dev.set_ctrl(EXPOSURE_AUTO, 0)?;
        self.framed(&cmd::exposure_mode_type(self.seq.next()))?;
        self.short(&cmd::exposure_mode_select(mode))?;
        State {
            exposure_mode: Some(mode),
            ..State::load()
        }
        .save()
    }

    /// Manual exposure: V4L2 auto_exposure 1, after which the image controls exposure_time
    /// (up to about 330 at 30 fps, one frame) and gain take effect. OBSBOT Center does this with
    /// frames 0x2442 / 0x2982, which are not decoded; the UVC way was checked on camera images
    /// (mean brightness 115 auto, 149 at exposure_time 100, 179 at 330).
    pub fn set_manual_exposure(&self) -> Result<()> {
        self.dev.set_ctrl(EXPOSURE_AUTO, 1)
    }

    pub fn factory_reset(&self) -> Result<()> {
        self.framed(&cmd::factory_reset(self.seq.next()))?;
        // the camera forgets its pose and exposure settings
        State::default().save()
    }

    // --- gimbal and zoom ---

    /// Moves the gimbal to an absolute pose (degrees; pan positive to the camera's left,
    /// tilt positive up). Switches AI tracking to static first.
    pub fn gimbal(&self, pan: Option<f32>, tilt: Option<f32>) -> Result<()> {
        // build first so invalid input fails before anything is sent
        let frame = cmd::gimbal_pose(pan, tilt.map(|t| -t + 0.0), self.seq.next())?;
        self.short(&cmd::ai_mode(AiMode::Static))?;
        self.framed(&frame)?;
        State::load().with_gimbal(pan, tilt).save()
    }

    /// Sets the zoom factor (1.0-4.0) through UVC zoom_absolute.
    pub fn set_zoom(&self, factor: f32) -> Result<()> {
        if !(1.0..=MAX_ZOOM).contains(&factor) {
            return Err(Error::Invalid("zoom factor is 1.0-4.0".into()));
        }
        let r = self
            .dev
            .ctrl_range(ZOOM_ABSOLUTE)?
            .ok_or_else(|| Error::Unsupported("no zoom control".into()))?;
        let share = (factor - 1.0) / (MAX_ZOOM - 1.0);
        let value = r.min + (share * (r.max - r.min) as f32).round() as i32;
        self.dev.set_ctrl(ZOOM_ABSOLUTE, value)
    }

    // --- image controls ---

    fn image_id(name: &str) -> Result<u32> {
        IMAGE_CONTROLS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, id)| *id)
            .ok_or_else(|| Error::Invalid(format!("unknown image control '{name}'")))
    }

    pub fn set_image(&self, name: &str, value: i32) -> Result<()> {
        let id = Self::image_id(name)?;
        let r = self
            .dev
            .ctrl_range(id)?
            .ok_or_else(|| Error::Unsupported(format!("camera has no '{name}' control")))?;
        if value < r.min || value > r.max {
            return Err(Error::Invalid(format!("{name} is {}-{}", r.min, r.max)));
        }
        self.dev.set_ctrl(id, value)
    }

    // --- presets ---

    /// Lists the presets stored in the camera, sorted by slot.
    pub fn presets(&self) -> Result<Vec<Preset>> {
        let block = self.dev.xu_get(0x0c)?;
        let slots = presets::parse_slots(&block);
        if slots.is_empty() {
            return Ok(Vec::new());
        }
        // writing the slot list back moves the entry cursor to the first preset
        self.dev.xu_set(0x0c, &block)?;
        let mut list = Vec::new();
        for _ in 0..slots.len() {
            match presets::parse_entry(&self.dev.xu_get(0x0d)?) {
                Some(p) => list.push(p),
                None => break,
            }
        }
        list.sort_by_key(|p| p.slot);
        Ok(list)
    }

    /// The pose to save: the remembered gimbal pose, else the one V4L2 last set (stale
    /// after tracking moved the gimbal), plus the live zoom factor.
    fn current_pose(&self) -> Result<Pose> {
        let state = State::load().gimbal;
        let v4l2 = |id| self.dev.get_ctrl(id).map(|v| v as f32 / 3600.0);
        let pan = match state.and_then(|g| g.pan) {
            Some(p) => p,
            None => v4l2(PAN_ABSOLUTE)?,
        };
        let tilt = match state.and_then(|g| g.tilt) {
            Some(t) => t,
            None => v4l2(TILT_ABSOLUTE)?,
        };
        Ok(Pose {
            pan,
            tilt,
            zoom: self.zoom_factor()?,
        })
    }

    /// Saves a preset: creates it, or updates the pose of an existing one. New presets get
    /// "Preset <n>" as name unless one is given. `pose` overrides the current pose.
    pub fn save_preset(&self, slot: u8, name: Option<&str>, pose: Option<Pose>) -> Result<Pose> {
        presets::check_slot(slot)?;
        let pose = match pose {
            Some(p) => p,
            None => self.current_pose()?,
        };
        let exists = self.presets()?.iter().any(|p| p.slot == slot);
        let seq = self.seq.next();
        self.framed(&if exists {
            presets::update(slot, pose, seq)?
        } else {
            presets::create(slot, pose, seq)?
        })?;
        let default = format!("Preset {}", slot + 1);
        if let Some(name) = name.or((!exists).then_some(default.as_str())) {
            sleep(PAIR_GAP);
            self.rename_preset(slot, name)?;
        }
        Ok(pose)
    }

    pub fn rename_preset(&self, slot: u8, name: &str) -> Result<()> {
        self.framed(&presets::rename(slot, name, self.seq.next())?)
    }

    pub fn delete_preset(&self, slot: u8) -> Result<()> {
        self.pair(presets::delete(slot, self.seq.next(), self.seq.next())?)
    }

    /// Recalls a preset and updates the remembered pose to it.
    pub fn recall_preset(&self, slot: u8) -> Result<()> {
        self.framed(&presets::recall(slot, self.seq.next())?)?;
        let static_mode = self.status()?.ai_mode == Some(AiMode::Static);
        let preset = self.presets()?.into_iter().find(|p| p.slot == slot);
        match preset {
            Some(p) if static_mode => State {
                gimbal: None,
                ..State::load()
            }
            .with_gimbal(Some(p.pose.pan), Some(p.pose.tilt))
            .save(),
            _ => State {
                gimbal: None,
                ..State::load()
            }
            .save(),
        }
    }
}
