// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! High-level camera operations (one device open per operation).

use crate::commands::{self as cmd, *};
use crate::device::{CtrlRange, Device};
use crate::error::{Error, Result};
use crate::frame::{Answer, Sequence, parse_answer, query_frame};
use crate::presets::{self, MAX_ZOOM, Pose, Preset, StartupActions};
use crate::state::{Startup, State};
use crate::status::{Focus, ModelInfo, Status, Target, Tracking, firmware_version};
use crate::streaming;
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

/// Commands obsbot4linux supports, for the `capabilities` list of `status --json`.
pub const CAPABILITIES: &[&str] = &[
    "sleep",
    "wake",
    "tracking",
    "speed",
    "hdr",
    "exposure",
    "zoom",
    "gimbal",
    "gimbal_speed",
    "gimbal_stop",
    "zoom_speed",
    "speed_mode",
    "speed_headroom",
    "framing",
    "fov",
    "led",
    "led_blink",
    "mirror",
    "invert",
    "auto_sleep",
    "sleep_background",
    "sleep_background_mirror",
    "audio",
    "audio_volume",
    "gesture",
    "hand",
    "zone",
    "zone_follow",
    "zone_return",
    "hand_return",
    "focus",
    "focus_af_mode",
    "focus_point",
    "wake_zoom",
    "wake_mode",
    "exposure_values",
    "startup",
    "image",
    "preset",
    "factory_reset",
    "target",
    "live_pose_on_demand",
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

/// Degrees in obsbot4linux's angles: pan positive = camera left, tilt positive = up.
#[derive(Debug, Serialize)]
pub struct LivePose {
    pub pan: f64,
    pub tilt: f64,
}

impl LivePose {
    /// Answer to 0x0043: bytes 8-9 are the pitch (positive down) and 10-11 the yaw (positive
    /// left), i16 in hundredths of a degree; reported to a tenth like the gimbal commands take.
    pub fn decode(d: &[u8]) -> Option<Self> {
        let i16_at = |i: usize| d.get(i..i + 2).map(|b| i16::from_le_bytes([b[0], b[1]]));
        let tenths = |v: f64| (v / 10.0).round() / 10.0;
        Some(LivePose {
            pan: tenths(i16_at(10)? as f64),
            tilt: tenths(-(i16_at(8)? as f64)) + 0.0, // no -0
        })
    }
}

#[derive(Debug, Serialize)]
pub struct FullStatus {
    /// Human name of the camera model, e.g. "Tiny 2 Lite"; null while no camera is connected.
    pub model: Option<&'static str>,
    /// V4L2 node the camera streams from (stable by-id path when available); null like `model`.
    pub device: Option<String>,
    /// Whether an application is streaming from the camera (kernel probe, see `streaming::is_streaming`).
    pub streaming: bool,
    /// `comm` names of those applications, sorted; empty when none.
    pub streaming_apps: Vec<String>,
    /// Always true here; the daemon reports false when the camera is gone.
    pub available: bool,
    /// Firmware version such as "6.2.8.1" (query 0x0408); null when the camera did not answer.
    pub firmware: Option<String>,
    /// Product and platform codes (query 0x1948); null when the camera did not answer.
    pub model_info: Option<ModelInfo>,
    pub status: Status,
    /// None when the camera did not answer query 0x0104.
    pub tracking: Option<Tracking>,
    pub zoom: Zoom,
    /// Live gimbal pose from query 0x0043 (gimbal controller), also while AI tracking moves it.
    pub pose: Option<LivePose>,
    /// Tracking target from query 0x01c4; None while Static or when the camera gave no usable answer.
    pub target: Option<Target>,
    pub image: BTreeMap<String, ImageControl>,
    /// Microphone volume 0-100 from query 0x80c2; null while asleep or when the camera did not
    /// answer.
    pub audio_volume: Option<u8>,
    /// Autofocus mode, manual position and focus window from query 0x3642; null like
    /// `audio_volume`.
    pub focus: Option<Focus>,
    pub state: StateView,
    /// Zone tracking area, follow and return position read from the camera; None when it did not
    /// answer the queries.
    pub zone: Option<Zone>,
    /// Hand tracking area and return position read from the camera; None when it did not answer.
    pub hand: Option<Hand>,
    /// Tracking speed mode and framing options read from the camera; None when it did not answer.
    pub framing: Option<Framing>,
    pub capabilities: Vec<&'static str>,
    /// What `gimbal`, the poses and `zone area` accept, so clients need not hardcode it.
    pub limits: Limits,
}

/// `limits` in the status: [min, max] in obsbot4linux's angles (pan positive = camera left, tilt
/// positive = up) and the zoom factor.
#[derive(Debug, Serialize, PartialEq)]
pub struct Limits {
    pub pan: [f32; 2],
    pub tilt: [f32; 2],
    pub zoom: [f32; 2],
}

pub const LIMITS: Limits = Limits {
    pan: [-GIMBAL_PAN_LIMIT, GIMBAL_PAN_LIMIT],
    tilt: [GIMBAL_TILT_MIN, GIMBAL_TILT_MAX],
    zoom: [1.0, MAX_ZOOM],
};

/// `state` in the status: what obsbot4linux remembers (state file), plus `zone_follow` and
/// `startup`, which are read from the camera (null when it did not answer). Kept under `state`
/// so existing clients keep working.
#[derive(Debug, Serialize)]
pub struct StateView {
    #[serde(flatten)]
    pub remembered: State,
    pub zone_follow: Option<bool>,
    pub startup: Option<Startup>,
}

/// Zone tracking settings as the camera reports them, in obsbot4linux's angles (pan positive =
/// camera left, tilt positive = up; the same as `zone area` takes).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Zone {
    pub pan_min: f32,
    pub pan_max: f32,
    pub tilt_min: f32,
    pub tilt_max: f32,
    /// Automatically follow new targets.
    pub follow: bool,
    /// Where the zone tracking returns to (`zone return-here` sets it, `zone return-go` moves there).
    #[serde(rename = "return")]
    pub return_pose: Pose,
}

/// Hand tracking settings as the camera reports them, in obsbot4linux's angles (pan positive =
/// camera left, tilt positive = up).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hand {
    pub area: HandArea,
    /// Where the camera goes after the hand leaves the tracking range; None while none is set.
    #[serde(rename = "return")]
    pub return_pose: Option<Pose>,
}

/// The angles around the camera's centre in which a hand is tracked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandArea {
    pub pan_min: f32,
    pub pan_max: f32,
    pub tilt_min: f32,
    pub tilt_max: f32,
}

/// Framing options of the human tracking as the camera reports them (queries `0x0984`, `0x0a84`,
/// `0x0b04`, `0x0b84`, `0x0c04`, `0x0c84`, `0x15c4`). An AI mode change overwrites most of them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Framing {
    pub speed_mode: SpeedMode,
    pub auto_zoom: bool,
    pub size: FramingSize,
    pub zoom_speed: AutoZoomSpeed,
    /// Framing offset as a fraction of the frame (-0.16 to 0.16).
    pub offset: Offset,
    pub part_mode: PartMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Offset {
    pub x: f32,
    pub y: f32,
}

/// The rarely changing readbacks; the daemon keeps them between full reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneInfo {
    pub zone: Option<Zone>,
    pub hand: Option<Hand>,
    pub startup: Option<Startup>,
    pub framing: Option<Framing>,
}

/// What does not change while the camera stays connected; the daemon reads it once per connect.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub firmware: Option<String>,
    pub model_info: Option<ModelInfo>,
}

/// The microphone volume from the answer to 0x80c2 (i16, 0..100); anything else is no volume.
fn decode_volume(d: &[u8]) -> Option<u8> {
    let v = i16::from_le_bytes(d.get(..2)?.try_into().ok()?);
    u8::try_from(v).ok().filter(|v| *v <= 100)
}

/// The steps of `exposure manual`, in the one order that is safe: range-check every value first
/// (so a bad one changes nothing), switch to manual exposure, and only then set the values. The
/// camera's shutter time written while the exposure is automatic freezes the automatic exposure
/// (image stays dark), so the values must never come before the switch.
fn manual_exposure(
    time: Option<i32>,
    gain: Option<i32>,
    mut check: impl FnMut(&str, i32) -> Result<()>,
    mut switch_to_manual: impl FnMut() -> Result<()>,
    mut set: impl FnMut(&str, i32) -> Result<()>,
) -> Result<()> {
    let values = [("exposure_time", time), ("gain", gain)];
    for (name, value) in values {
        if let Some(v) = value {
            check(name, v)?;
        }
    }
    switch_to_manual()?;
    for (name, value) in values {
        if let Some(v) = value {
            set(name, v)?;
        }
    }
    Ok(())
}

/// One sequence for the whole process, so frames from separate `Camera` handles (the daemon
/// opens the device per operation) never repeat a number.
/// The answer to 0x0b84 is the 0-2 the setter takes, but a camera that was never set reports its
/// default as the speed itself, 10 (read back from a Tiny 2 Lite); 30 and 50 are the other two.
fn auto_zoom_speed_reading(v: u8) -> Option<AutoZoomSpeed> {
    match v {
        0 | 10 => Some(AutoZoomSpeed::Slow),
        1 | 30 => Some(AutoZoomSpeed::Medium),
        2 | 50 => Some(AutoZoomSpeed::Fast),
        _ => None,
    }
}

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
        self.query_to(0x04, command)
    }

    fn query_to(&self, receiver: u8, command: u16) -> Result<Vec<u8>> {
        let seq = self.seq.next();
        self.framed(&query_frame(receiver, command, seq))?;
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

    /// Query 0x0043 to the gimbal controller (receiver 3, `gimbalGetAttitudeInfoR` in OBSBOT
    /// Center): the live pose, also while AI tracking moves the gimbal. Not the AI's 0x6604:
    /// every 0x6604 arms a flag in the camera that makes the next zone tracking return switch to
    /// Static (#67).
    pub fn live_pose(&self) -> Result<LivePose> {
        LivePose::decode(&self.query_to(0x03, 0x0043)?).ok_or(Error::NoAnswer(0x0043))
    }

    /// Query 0x01c4: the tracking target. Only asked while AI tracking is on; Static answers
    /// a single `00`, which is None like any reply that does not decode.
    pub fn target(&self) -> Result<Option<Target>> {
        Ok(Target::decode(&self.query(0x01c4)?))
    }

    /// Query 0x80c2: the microphone volume (i16, 0..100).
    pub fn audio_volume(&self) -> Result<u8> {
        decode_volume(&self.query_to(0x02, 0x80c2)?).ok_or(Error::NoAnswer(0x80c2))
    }

    /// Query 0x3642: autofocus mode, manual position and focus window.
    pub fn focus(&self) -> Result<Focus> {
        Focus::decode(&self.query_to(0x02, 0x3642)?).ok_or(Error::NoAnswer(0x3642))
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

    /// Reads a float answer of `command` (zone area queries).
    fn query_f32(&self, command: u16) -> Result<f32> {
        let d = self.query(command)?;
        d.get(..4)
            .and_then(|s| s.try_into().ok())
            .map(f32::from_le_bytes)
            .filter(|v| v.is_finite())
            .ok_or(Error::NoAnswer(command))
    }

    /// Queries `0x4c44` / `0x4cc4` (pan min / max), `0x4d44` / `0x4dc4` (tilt min / max, camera
    /// pitch: positive down, so min and max swap and flip), `0x4e44` (follow), `0x4ec4` (return
    /// position). All of them are queries OBSBOT Center sends to this camera.
    pub fn zone(&self) -> Result<Zone> {
        let pitch_min = self.query_f32(0x4d44)?;
        let pitch_max = self.query_f32(0x4dc4)?;
        let follow = self.query(0x4e44)?;
        let (_, return_pose) =
            presets::parse_pose(&self.query(0x4ec4)?).ok_or(Error::NoAnswer(0x4ec4))?;
        Ok(Zone {
            pan_min: self.query_f32(0x4c44)?,
            pan_max: self.query_f32(0x4cc4)?,
            tilt_min: -pitch_max + 0.0,
            tilt_max: -pitch_min + 0.0,
            follow: follow.first().ok_or(Error::NoAnswer(0x4e44))? != &0,
            return_pose,
        })
    }

    /// Queries `0x2004` (hand area: four f32, yaw min / max, pitch min / max; pitch is positive
    /// down, so min and max swap and flip like for the zone) and `0x2504` (return position, leading
    /// `i32` -1 while none is set). Both are queries OBSBOT Center sends to this camera.
    pub fn hand(&self) -> Result<Hand> {
        let area = self.query(0x2004)?;
        let float = |i: usize| -> Result<f32> {
            area.get(i * 4..i * 4 + 4)
                .and_then(|s| s.try_into().ok())
                .map(f32::from_le_bytes)
                .filter(|v| v.is_finite())
                .ok_or(Error::NoAnswer(0x2004))
        };
        let (lead, pose) =
            presets::parse_pose(&self.query(0x2504)?).ok_or(Error::NoAnswer(0x2504))?;
        Ok(Hand {
            area: HandArea {
                pan_min: float(0)?,
                pan_max: float(1)?,
                tilt_min: -float(3)? + 0.0,
                tilt_max: -float(2)? + 0.0,
            },
            return_pose: (lead != u32::MAX).then_some(pose),
        })
    }

    /// Query `0x3884`: the start pose of the initial state. The leading number is 0-2 for a
    /// preset slot and 3 for an own pose.
    pub fn startup(&self) -> Result<Startup> {
        let (source, pose) =
            presets::parse_pose(&self.query(0x3884)?).ok_or(Error::NoAnswer(0x3884))?;
        Ok(Startup {
            preset: u8::try_from(source).ok().filter(|s| *s < presets::SLOTS),
            pan: pose.pan,
            tilt: pose.tilt,
            zoom: pose.zoom,
        })
    }

    /// Query `0x3e84`: the picture settings of the initial state.
    pub fn startup_actions(&self) -> Result<StartupActions> {
        StartupActions::decode(&self.query(0x3e84)?).ok_or(Error::NoAnswer(0x3e84))
    }

    /// Queries 0x0408 and 0x1948 to receiver 0x0d (the camera's upgrade process, which answers
    /// them as OBSBOT Center asks at startup). Not 0x1ac8 there: Center names it a shell command.
    pub fn identity(&self) -> Identity {
        Identity {
            firmware: self
                .query_to(0x0d, 0x0408)
                .ok()
                .and_then(|d| firmware_version(&d)),
            model_info: self
                .query_to(0x0d, 0x1948)
                .ok()
                .and_then(|d| ModelInfo::decode(&d)),
        }
    }

    /// Reads one byte of the answer to `command` (the framing options are all u8).
    fn query_u8(&self, command: u16) -> Result<u8> {
        self.query(command)?
            .first()
            .copied()
            .ok_or(Error::NoAnswer(command))
    }

    /// The tracking speed mode and framing options (see [`Framing`]). An answer outside the
    /// documented values counts as no answer.
    pub fn framing(&self) -> Result<Framing> {
        let byte = |command: u16| self.query_u8(command);
        let bad = |command: u16| Error::NoAnswer(command);
        Ok(Framing {
            speed_mode: SpeedMode::from_u8(byte(0x0984)?).ok_or(bad(0x0984))?,
            auto_zoom: byte(0x0a84)? != 0,
            size: FramingSize::from_u8(byte(0x0b04)?).ok_or(bad(0x0b04))?,
            zoom_speed: auto_zoom_speed_reading(byte(0x0b84)?).ok_or(bad(0x0b84))?,
            offset: Offset {
                x: self.query_f32(0x0c04)?,
                y: self.query_f32(0x0c84)?,
            },
            part_mode: PartMode::from_u8(byte(0x15c4)?).ok_or(bad(0x15c4))?,
        })
    }

    pub fn zone_info(&self) -> ZoneInfo {
        ZoneInfo {
            zone: self.zone().ok(),
            hand: self.hand().ok(),
            startup: self.startup().ok(),
            framing: self.framing().ok(),
        }
    }

    pub fn full_status(&self) -> Result<FullStatus> {
        self.full_status_with(None, None)
    }

    /// Like [`Camera::full_status`], with the zone readbacks and the identity given instead of
    /// asked for again.
    pub fn full_status_with(
        &self,
        known: Option<ZoneInfo>,
        identity: Option<Identity>,
    ) -> Result<FullStatus> {
        let status = self.status()?;
        let tracking = match self.tracking() {
            Ok(t) => Some(t),
            Err(Error::NoAnswer(_) | Error::Rejected(..)) => None,
            Err(e) => return Err(e),
        };
        let factor = self.zoom_factor().ok();
        let absolute = self.dev.get_ctrl(ZOOM_ABSOLUTE).ok();
        let detected = self.dev.detected();
        let streaming_apps = streaming::apps_using(&detected.device);
        // an asleep camera does not answer these; asking would only wait out the polling
        let (audio_volume, focus) = if status.asleep {
            (None, None)
        } else {
            (self.audio_volume().ok(), self.focus().ok())
        };
        let target = if status.ai_mode == Some(AiMode::Static) {
            None
        } else {
            self.target().ok().flatten()
        };
        let ZoneInfo {
            zone,
            hand,
            startup,
            framing,
        } = known.unwrap_or_else(|| self.zone_info());
        let Identity {
            firmware,
            model_info,
        } = identity.unwrap_or_else(|| self.identity());
        Ok(FullStatus {
            model: Some(detected.model.name),
            device: Some(detected.device.to_string_lossy().into_owned()),
            streaming: streaming::is_streaming(&detected.device),
            streaming_apps,
            available: true,
            firmware,
            model_info,
            status,
            tracking,
            zoom: Zoom { factor, absolute },
            pose: self.live_pose().ok(),
            target,
            image: self.image_controls()?,
            audio_volume,
            focus,
            state: StateView {
                remembered: State::load(),
                zone_follow: zone.as_ref().map(|z| z.follow),
                startup,
            },
            zone,
            hand,
            framing,
            capabilities: CAPABILITIES.to_vec(),
            limits: LIMITS,
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
    pub fn set_speed_mode(&self, mode: SpeedMode) -> Result<()> {
        self.framed(&cmd::speed_mode(mode, self.seq.next()))
    }
    pub fn set_auto_zoom(&self, on: bool) -> Result<()> {
        self.framed(&cmd::auto_zoom(on, self.seq.next()))
    }
    pub fn set_framing_size(&self, size: FramingSize) -> Result<()> {
        self.framed(&cmd::framing_size(size, self.seq.next()))
    }
    pub fn set_auto_zoom_speed(&self, speed: AutoZoomSpeed) -> Result<()> {
        self.framed(&cmd::auto_zoom_speed(speed, self.seq.next()))
    }
    pub fn set_part_mode(&self, mode: PartMode) -> Result<()> {
        self.framed(&cmd::part_mode(mode, self.seq.next()))
    }
    /// Sets the framing offset; both frames are built before the first is sent.
    pub fn set_framing_offset(&self, x: Option<f32>, y: Option<f32>) -> Result<()> {
        if x.is_none() && y.is_none() {
            return Err(Error::Invalid("give --x and/or --y".into()));
        }
        let frames = [
            x.map(|x| cmd::framing_offset(false, x, self.seq.next()))
                .transpose()?,
            y.map(|y| cmd::framing_offset(true, y, self.seq.next()))
                .transpose()?,
        ];
        let mut first = true;
        for frame in frames.iter().flatten() {
            if !first {
                sleep(PAIR_GAP);
            }
            first = false;
            self.framed(frame)?;
        }
        Ok(())
    }
    pub fn set_hdr(&self, on: bool) -> Result<()> {
        self.short(&cmd::hdr(on))
    }
    pub fn set_auto_sleep(&self, seconds: Option<u16>) -> Result<()> {
        let payload = match seconds {
            Some(s) => cmd::auto_sleep(
                i16::try_from(s)
                    .map_err(|_| Error::Invalid("auto-sleep time is at most 32767 s".into()))?,
            ),
            None => cmd::auto_sleep_off(self.status()?.auto_sleep_seconds)?,
        };
        self.short(&payload)
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
    pub fn set_led_blink(&self, on: bool) -> Result<()> {
        self.short(&cmd::led_blink(on))
    }
    /// Zoom after waking up: factor 1.0-4.0, stored as 0-100 percent (status `wake_up_zoom_percent`).
    pub fn set_wake_zoom(&self, factor: f32) -> Result<u8> {
        let percent = cmd::wake_zoom_percent(factor)?;
        self.short(&cmd::wake_zoom(percent)?)?;
        Ok(percent)
    }
    /// Work mode the camera starts in (status `default_work_mode`); the current AI mode stays.
    pub fn set_default_work_mode(&self, mode: AiMode) -> Result<()> {
        self.short(&cmd::default_work_mode(mode))
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
    /// Microphone volume 0-100; the pickup distance in the status follows (near below 50,
    /// standard 50-84, far from 85).
    pub fn set_audio_volume(&self, volume: u8) -> Result<()> {
        self.framed(&cmd::audio_volume(volume, self.seq.next())?)
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

    /// Zone area in obsbot4linux's angles (pan positive = left, tilt positive = up), converted to the
    /// camera's (pitch positive = down).
    pub fn set_zone_area(&self, pan: (f32, f32), tilt: (f32, f32)) -> Result<()> {
        let seqs = [0; 4].map(|_| self.seq.next());
        for f in cmd::zone_area(pan, (-tilt.1, -tilt.0), seqs)? {
            self.framed(&f)?;
        }
        Ok(())
    }

    /// Sets the current pose (live pose 0x0043 and zoom 0x6804) as the zone return position and
    /// returns it.
    pub fn set_zone_return_here(&self) -> Result<Pose> {
        let live = self.live_pose()?;
        let pose = Pose {
            pan: live.pan as f32,
            tilt: live.tilt as f32 + 0.0, // no -0
            zoom: self.zoom_factor()?.clamp(1.0, MAX_ZOOM),
        };
        self.framed(&presets::zone_return_set(pose, self.seq.next())?)?;
        Ok(pose)
    }

    /// Sets the current pose as the hand-tracking return position and returns it.
    pub fn set_hand_return_here(&self) -> Result<Pose> {
        let live = self.live_pose()?;
        let pose = Pose {
            pan: live.pan as f32,
            tilt: live.tilt as f32 + 0.0, // no -0
            zoom: self.zoom_factor()?.clamp(1.0, MAX_ZOOM),
        };
        self.framed(&presets::hand_return_set(pose, self.seq.next())?)?;
        Ok(pose)
    }

    /// Moves the gimbal to the hand-tracking return position.
    pub fn hand_return_go(&self) -> Result<()> {
        self.framed(&cmd::hand_return_go(self.seq.next()))
    }

    /// Clears the hand-tracking return position.
    pub fn hand_return_reset(&self) -> Result<()> {
        self.framed(&cmd::hand_return_reset(self.seq.next()))
    }

    /// Moves the gimbal to the zone return position.
    pub fn zone_return_go(&self) -> Result<()> {
        self.framed(&cmd::zone_return_go(self.seq.next()))
    }

    /// Moves the gimbal to the boot pose (the start pose of the initial state).
    pub fn startup_go(&self) -> Result<()> {
        self.short(&cmd::startup_go())
    }

    pub fn set_zone_follow(&self, on: bool) -> Result<()> {
        self.framed(&cmd::zone_follow(on, self.seq.next()))
    }

    /// Autofocus mode: face or global (status byte 0x0d follows).
    pub fn set_autofocus_mode(&self, face: bool) -> Result<()> {
        self.framed(&cmd::autofocus_mode(face, self.seq.next()))
    }

    /// The camera's own autofocus mode (status byte 0x0e is 1 only in continuous).
    pub fn set_autofocus_select(&self, mode: AutofocusMode) -> Result<()> {
        self.framed(&cmd::autofocus_mode_select(mode, self.seq.next()))
    }

    /// Focuses once on a point of the picture (x and y 0..1 from the top left); the camera
    /// switches to single autofocus.
    pub fn focus_point(&self, x: f32, y: f32) -> Result<()> {
        self.framed(&cmd::focus_point(x, y, self.seq.next())?)
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
    pub fn set_startup(&self, pose: Pose, preset: Option<u8>) -> Result<()> {
        let source = preset.map_or(3, u32::from);
        let pose_frame = presets::startup_pose(source, pose, self.seq.next())?;
        self.framed(&pose_frame)?;
        sleep(PAIR_GAP);
        self.framed(&presets::startup_picture_settings(self.seq.next()))
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
        // back to automatic exposure in case it was set to manual. Center sends only the
        // metering here; its frame 0x2982 sets a manual exposure value and freezes the automatic
        // exposure (image stays dark), so it must not be sent.
        self.dev.set_ctrl(EXPOSURE_AUTO, 0)?;
        self.short(&cmd::exposure_mode_select(mode))?;
        State {
            exposure_mode: Some(mode),
            ..State::load()
        }
        .save()
    }

    /// Manual exposure: V4L2 auto_exposure 1, after which the image controls exposure_time
    /// (100 us units, 1-2500; the camera takes the step at or below it) and gain take effect.
    /// With `time` and `gain` they are set right after, only once both are in range. The camera's
    /// own frames 0x2442 / 0x2982 do the same (checked by reading the mode and shutter back with
    /// 0x2942 and by the image brightness) but leave these UVC values stale, so the UVC way is
    /// the one that keeps `status.image` true.
    pub fn set_manual_exposure(&self, time: Option<i32>, gain: Option<i32>) -> Result<()> {
        manual_exposure(
            time,
            gain,
            |name, v| self.image_id_in_range(name, v).map(|_| ()),
            || self.dev.set_ctrl(EXPOSURE_AUTO, 1),
            |name, v| self.set_image(name, v),
        )
    }

    pub fn factory_reset(&self) -> Result<()> {
        self.framed(&cmd::factory_reset(self.seq.next()))?;
        // the camera forgets its pose and exposure settings
        State::default().save()
    }

    // --- gimbal and zoom ---

    /// Moves the gimbal to an absolute pose (degrees; pan positive to the camera's left,
    /// tilt positive up). Switches AI tracking to static first. With `speed` (degrees per second)
    /// the move takes as long as that speed needs instead of going as fast as the motors do.
    pub fn gimbal(&self, pan: Option<f32>, tilt: Option<f32>, speed: Option<f32>) -> Result<()> {
        // build first so invalid input fails before anything is sent
        let seq = self.seq.next();
        let tilt_wire = tilt.map(|t| -t + 0.0);
        let frame = match speed {
            Some(s) => cmd::gimbal_pose_speed(pan, tilt_wire, s, seq)?,
            None => cmd::gimbal_pose(pan, tilt_wire, seq)?,
        };
        self.short(&cmd::ai_mode(AiMode::Static))?;
        self.framed(&frame)?;
        State::load().with_gimbal(pan, tilt).save()
    }

    /// Starts turning the gimbal (degrees per second, pan positive = camera left, tilt positive
    /// = up) until `gimbal_stop` or a rate of 0. Switches AI tracking to static first.
    pub fn gimbal_rate(&self, pan: f32, tilt: f32) -> Result<()> {
        // the camera reverses the yaw of this command for mirror xor invert (neither is in the way
        // `gimbal --pan` turns)
        let (mirror, invert) = (self.status()?.mirror, self.tracking()?.invert);
        let frame = cmd::gimbal_rate(pan, tilt, mirror, invert, self.seq.next())?;
        self.short(&cmd::ai_mode(AiMode::Static))?;
        self.framed(&frame)?;
        self.forget_gimbal()
    }

    /// Stops all gimbal axes.
    pub fn gimbal_stop(&self) -> Result<()> {
        self.framed(&cmd::gimbal_stop(self.seq.next()))?;
        self.forget_gimbal()
    }

    /// Turns the gimbal so that a point of the picture (x, y from 0.0 to 1.0, from the top left)
    /// moves to the centre. Switches AI tracking to static first.
    pub fn gimbal_center(&self, x: f32, y: f32) -> Result<()> {
        let frame = cmd::view_center(x, y, self.seq.next())?;
        self.short(&cmd::ai_mode(AiMode::Static))?;
        self.framed(&frame)?;
        self.forget_gimbal()
    }

    /// The remembered pose is no longer true after a move the camera worked out itself.
    fn forget_gimbal(&self) -> Result<()> {
        State {
            gimbal: None,
            ..State::load()
        }
        .save()
    }

    /// Zooms to a factor at a speed through the AI path (the UVC zoom control jumps).
    pub fn zoom_to(&self, factor: f32, speed: i32) -> Result<()> {
        self.framed(&cmd::zoom_to(speed, factor, self.seq.next())?)
    }

    /// Stops a zoom in progress.
    pub fn zoom_stop(&self) -> Result<()> {
        self.framed(&cmd::zoom_stop(self.seq.next()))
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

    /// The control id of `name` when the camera has it and `value` is in its range.
    fn image_id_in_range(&self, name: &str, value: i32) -> Result<u32> {
        let id = Self::image_id(name)?;
        let r = self
            .dev
            .ctrl_range(id)?
            .ok_or_else(|| Error::Unsupported(format!("camera has no '{name}' control")))?;
        if value < r.min || value > r.max {
            return Err(Error::Invalid(format!("{name} is {}-{}", r.min, r.max)));
        }
        Ok(id)
    }

    pub fn set_image(&self, name: &str, value: i32) -> Result<()> {
        let id = self.image_id_in_range(name, value)?;
        // a shutter time sent while the exposure is automatic freezes the automatic exposure
        if name == "exposure_time" && self.dev.get_ctrl(EXPOSURE_AUTO)? != 1 {
            return Err(Error::Invalid(
                "exposure_time needs manual exposure: use `exposure manual --time <v>`".into(),
            ));
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
            zoom: self.zoom_factor()?.clamp(1.0, MAX_ZOOM),
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

#[cfg(test)]
mod full_status_tests {
    #[test]
    fn hand_serialises_area_and_a_null_return() {
        let mut h = Hand {
            area: HandArea {
                pan_min: -45.0,
                pan_max: 45.0,
                tilt_min: -30.0,
                tilt_max: 30.0,
            },
            return_pose: None,
        };
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v["area"]["tilt_max"], 30.0);
        assert!(v["return"].is_null(), "no return position set");
        h.return_pose = Some(Pose {
            pan: 20.0,
            tilt: 5.0,
            zoom: 1.0,
        });
        assert_eq!(
            serde_json::to_value(h).unwrap()["return"],
            serde_json::json!({"pan": 20.0, "tilt": 5.0, "zoom": 1.0})
        );
    }

    #[test]
    fn zone_serialises_with_return_as_key() {
        let z = Zone {
            pan_min: -10.0,
            pan_max: 10.0,
            tilt_min: -5.0,
            tilt_max: 5.0,
            follow: true,
            return_pose: Pose {
                pan: 0.0,
                tilt: 0.0,
                zoom: 1.0,
            },
        };
        assert_eq!(
            serde_json::to_value(z).unwrap(),
            serde_json::json!({"pan_min": -10.0, "pan_max": 10.0, "tilt_min": -5.0, "tilt_max": 5.0,
                "follow": true, "return": {"pan": 0.0, "tilt": 0.0, "zoom": 1.0}})
        );
    }

    use super::*;

    #[test]
    fn model_and_device_are_top_level_keys() {
        let full = FullStatus {
            model: Some("Tiny 2 Lite"),
            device: Some("/dev/v4l/by-id/x-video-index0".into()),
            streaming: true,
            streaming_apps: vec!["ffmpeg".into()],
            available: true,
            firmware: Some("6.2.8.1".into()),
            model_info: Some(ModelInfo {
                product: 8,
                platform: 8,
            }),
            status: Status::decode(&[0; 60]),
            tracking: None,
            zoom: Zoom {
                factor: None,
                absolute: None,
            },
            pose: None,
            target: None,
            image: BTreeMap::new(),
            audio_volume: Some(10),
            focus: None,
            state: StateView {
                remembered: State::default(),
                zone_follow: Some(true),
                startup: None,
            },
            zone: None,
            hand: None,
            framing: Some(Framing {
                speed_mode: SpeedMode::VeryFast,
                auto_zoom: true,
                size: FramingSize::MediumCloseUp,
                zoom_speed: AutoZoomSpeed::Fast,
                offset: Offset {
                    x: 0.125,
                    y: -0.0625,
                },
                part_mode: PartMode::HeadRemoval,
            }),
            capabilities: CAPABILITIES.to_vec(),
            limits: LIMITS,
        };
        let v = serde_json::to_value(full).unwrap();
        // the widget reads these names; they are the snake_case of the CLI names
        assert_eq!(
            v["framing"],
            serde_json::json!({"speed_mode": "very_fast", "auto_zoom": true,
                "size": "medium_close_up", "zoom_speed": "fast",
                "offset": {"x": 0.125, "y": -0.0625}, "part_mode": "head_removal"})
        );
        assert_eq!(v["model"], "Tiny 2 Lite");
        assert_eq!(v["device"], "/dev/v4l/by-id/x-video-index0");
        assert_eq!(v["streaming"], true);
        assert_eq!(v["streaming_apps"], serde_json::json!(["ffmpeg"]));
        assert!(v["target"].is_null());
        assert_eq!(v["firmware"], "6.2.8.1");
        assert_eq!(
            v["model_info"],
            serde_json::json!({"product": 8, "platform": 8})
        );
        // zone_follow stays under state, next to the remembered values
        assert_eq!(v["state"]["zone_follow"], true);
        assert!(v["state"]["startup"].is_null() && v["zone"].is_null() && v["hand"].is_null());
        // the widget takes its d-pad and zone slider ranges from here
        assert_eq!(
            v["limits"],
            serde_json::json!({"pan": [-150.0, 150.0], "tilt": [-70.0, 90.0], "zoom": [1.0, 4.0]})
        );
        let t = Target {
            detected: true,
            coords: [0.25, 0.5, 0.75, 1.0],
        };
        assert_eq!(
            serde_json::to_value(t).unwrap(),
            serde_json::json!({"detected": true, "box": [0.25, 0.5, 0.75, 1.0]})
        );
        // the keys the widget already reads stay where they were
        assert_eq!(v["available"], true);
        assert!(v["status"].is_object() && v["capabilities"].is_array());
    }
}

#[cfg(test)]
mod framing_tests {
    use super::auto_zoom_speed_reading;
    use crate::commands::AutoZoomSpeed;

    #[test]
    fn the_zoom_speed_is_read_as_index_or_as_the_default_speed() {
        // a camera nobody set reports 10 (read back from a Tiny 2 Lite); after the setter the index
        assert_eq!(auto_zoom_speed_reading(10), Some(AutoZoomSpeed::Slow));
        assert_eq!(auto_zoom_speed_reading(0), Some(AutoZoomSpeed::Slow));
        assert_eq!(auto_zoom_speed_reading(1), Some(AutoZoomSpeed::Medium));
        assert_eq!(auto_zoom_speed_reading(30), Some(AutoZoomSpeed::Medium));
        assert_eq!(auto_zoom_speed_reading(2), Some(AutoZoomSpeed::Fast));
        assert_eq!(auto_zoom_speed_reading(50), Some(AutoZoomSpeed::Fast));
        assert_eq!(auto_zoom_speed_reading(7), None);
    }
}

#[cfg(test)]
mod live_pose_tests {
    use super::LivePose;

    // 0x0043 answer of a Tiny 2 Lite in its sleep pose; 0x6604 read pan -0.3 / tilt -85.0 there.
    const SLEEP_POSE: &str = "38004e24043000003321e1ff0000cf00fafffd20010000000101110000000000";

    #[test]
    fn reads_the_same_pose_as_the_ai_query_it_replaces() {
        let d: Vec<u8> = (0..SLEEP_POSE.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&SLEEP_POSE[i..i + 2], 16).unwrap())
            .collect();
        let p = LivePose::decode(&d).unwrap();
        assert_eq!((p.pan, p.tilt), (-0.3, -85.0));
    }

    #[test]
    fn signs_follow_obsbot4linux_angles() {
        // pitch +15.00 (down) and yaw +20.00 (left) must read as tilt -15, pan +20
        let mut d = [0u8; 32];
        d[8..10].copy_from_slice(&1500i16.to_le_bytes());
        d[10..12].copy_from_slice(&2000i16.to_le_bytes());
        let p = LivePose::decode(&d).unwrap();
        assert_eq!((p.pan, p.tilt), (20.0, -15.0));
    }

    #[test]
    fn a_short_answer_is_no_pose() {
        assert!(LivePose::decode(&[0u8; 11]).is_none());
    }
}

#[cfg(test)]
mod picture_and_sound_tests {
    use super::*;
    use std::cell::RefCell;

    fn rec<T>(log: &RefCell<Vec<T>>, entry: T) -> Result<()> {
        log.borrow_mut().push(entry);
        Ok(())
    }

    #[test]
    fn volume_answer_must_be_0_to_100() {
        assert_eq!(decode_volume(&[0x0a, 0x00]), Some(10));
        assert_eq!(decode_volume(&[0x64, 0x00]), Some(100));
        assert_eq!(decode_volume(&[0x65, 0x00]), None, "101");
        assert_eq!(decode_volume(&[0xff, 0xff]), None, "-1");
        assert_eq!(decode_volume(&[0x0a, 0x01]), None, "266, not the low byte");
        assert_eq!(decode_volume(&[0x0a]), None, "short");
    }

    #[test]
    fn exposure_values_come_after_the_switch_to_manual() {
        let log = RefCell::new(Vec::new());
        manual_exposure(
            Some(100),
            Some(7),
            |n, v| rec(&log, format!("check {n} {v}")),
            || rec(&log, "manual".into()),
            |n, v| rec(&log, format!("set {n} {v}")),
        )
        .unwrap();
        assert_eq!(
            *log.borrow(),
            [
                "check exposure_time 100",
                "check gain 7",
                "manual",
                "set exposure_time 100",
                "set gain 7"
            ]
        );
    }

    #[test]
    fn a_bad_value_changes_nothing() {
        let log = RefCell::new(Vec::new());
        let r = manual_exposure(
            Some(9999),
            None,
            |_, _| Err(Error::Invalid("out of range".into())),
            || rec(&log, "manual"),
            |_, _| rec(&log, "set"),
        );
        assert!(r.is_err());
        assert!(log.borrow().is_empty(), "no switch, no value");
    }
}
