// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! Byte builders for the documented commands, plus the setting enums.
//!
//! Short commands go to unit 2 selector 6 (`<cmd> <len> <value>`), framed commands to
//! selector 2 (see [`crate::frame`]). Only ids from the protocol reference are used.

use crate::error::{Error, Result};
use crate::frame::{FRAME_SIZE, frame};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

macro_rules! setting {
    ($(#[$m:meta])* $name:ident { $($v:ident = $n:expr),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($v),+ }
        impl $name {
            pub fn from_u8(value: u8) -> Option<Self> {
                match value { $($n => Some(Self::$v),)+ _ => None }
            }
            pub fn to_u8(self) -> u8 {
                match self { $(Self::$v => $n),+ }
            }
        }
    };
}

setting!(FieldOfView { Wide = 0, Medium = 1, Narrow = 2 });
setting!(NoiseReduction { Off = 0, Weak = 1, Medium = 2, Strong = 3 });
setting!(PickupDistance { Near = 0, Standard = 1, Far = 2 });
setting!(TrackingSpeed { Standard = 0, Sport = 2 });
setting!(SleepBackground { Black = 0, Image = 1 });
setting!(HandSide { Right = 0, Left = 1 });
setting!(
    /// Automatic exposure mode. The camera reports it only as status byte 0x07 (face);
    /// t2l also remembers the last one set. Manual exposure is not mapped yet.
    ExposureMode { Global = 0, Face = 1 }
);

/// AI tracking mode as (mode, sub-mode), the pair of status bytes 0x18 / 0x1c.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AiMode {
    Static,
    Normal,
    UpperBody,
    CloseUp,
    Headless,
    LowerBody,
    Hand,
    Group,
}

impl AiMode {
    pub fn to_bytes(self) -> [u8; 2] {
        match self {
            Self::Static => [0, 0],
            Self::Normal => [2, 0],
            Self::UpperBody => [2, 1],
            Self::CloseUp => [2, 2],
            Self::Headless => [2, 3],
            Self::LowerBody => [2, 4],
            Self::Hand => [3, 0],
            Self::Group => [1, 0],
        }
    }

    /// Decodes the mode/sub-mode pair of the status block.
    pub fn from_status(mode: u8, sub: u8) -> Option<Self> {
        match (mode, sub) {
            (0, 0) => Some(Self::Static),
            (2, 0) => Some(Self::Normal),
            (2, 1) => Some(Self::UpperBody),
            (2, 2) => Some(Self::CloseUp),
            (2, 3) => Some(Self::Headless),
            (2, 4) => Some(Self::LowerBody),
            (3, 0) => Some(Self::Hand),
            (1, 0) => Some(Self::Group),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    TargetLock,
    Zoom,
    DynamicZoom,
    DirectionChange,
}

// Receivers: 0x04 for almost everything, 0x02 for power and exposure.
const TRACKING: u8 = 0x04;
const POWER: u8 = 0x02;

fn invalid(what: &str) -> Error {
    Error::Invalid(what.into())
}

// --- short commands (selector 6) ---

pub fn ai_mode(mode: AiMode) -> [u8; 4] {
    let [m, n] = mode.to_bytes();
    [0x16, 0x02, m, n]
}
pub fn hdr(on: bool) -> [u8; 3] {
    [0x01, 0x01, on as u8]
}
/// Auto-sleep time in seconds; negative turns auto-sleep off (the time is kept).
pub fn auto_sleep(seconds: i16) -> [u8; 4] {
    let [lo, hi] = seconds.to_le_bytes();
    [0x0b, 0x02, lo, hi]
}
pub fn sleep_background(background: SleepBackground) -> [u8; 4] {
    [0x0e, 0x02, 0x00, background.to_u8()]
}
pub fn sleep_background_mirror(on: bool) -> [u8; 4] {
    [0x0e, 0x02, 0x02, on as u8]
}
pub fn led(brightness: u8) -> Result<[u8; 3]> {
    if brightness > 3 {
        return Err(invalid("LED brightness is 0-3"));
    }
    Ok([0x1a, 0x01, brightness])
}
pub fn field_of_view(fov: FieldOfView) -> [u8; 3] {
    [0x04, 0x01, fov.to_u8()]
}
pub fn mirror(on: bool) -> [u8; 3] {
    [0x14, 0x01, on as u8]
}
pub fn mic_in_sleep(on: bool) -> [u8; 3] {
    [0x13, 0x01, on as u8]
}
pub fn noise_reduction(level: NoiseReduction) -> [u8; 3] {
    [0x0a, 0x01, level.to_u8()]
}
pub fn auto_gain(on: bool) -> [u8; 3] {
    [0x17, 0x01, on as u8]
}
pub fn pickup_distance(distance: PickupDistance) -> [u8; 3] {
    [0x1b, 0x01, distance.to_u8()]
}
/// Microphone on/off. Off makes the camera restart (about 18 s).
pub fn microphone(on: bool) -> [u8; 3] {
    [0x1c, 0x01, on as u8]
}
/// Second half of the exposure mode: global or face (capture 12: \`03 01 00\` / \`03 01 01\`).
pub fn exposure_mode_select(mode: ExposureMode) -> [u8; 3] {
    [0x03, 0x01, mode.to_u8()]
}

// --- framed commands (selector 2) ---

pub fn tracking_speed(speed: TrackingSpeed, seq: u16) -> [u8; FRAME_SIZE] {
    frame(TRACKING, 0x0cc4, &[speed.to_u8()], seq)
}
pub fn gesture(gesture: Gesture, on: bool, seq: u16) -> [u8; FRAME_SIZE] {
    let command = match gesture {
        Gesture::TargetLock => 0x30c4,
        Gesture::Zoom => 0x3144,
        Gesture::DynamicZoom => 0x3344,
        Gesture::DirectionChange => 0x33c4,
    };
    frame(TRACKING, command, &[on as u8], seq)
}
pub fn gesture_zoom_ratio(ratio: f32, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    if !(1.0..=4.0).contains(&ratio) {
        return Err(invalid("gesture zoom ratio is 1.0-4.0"));
    }
    Ok(frame(TRACKING, 0x3244, &ratio.to_le_bytes(), seq))
}
pub fn hand_side(side: HandSide, seq: u16) -> [u8; FRAME_SIZE] {
    frame(TRACKING, 0x2244, &[side.to_u8()], seq)
}
pub fn invert(on: bool, seq: u16) -> [u8; FRAME_SIZE] {
    frame(TRACKING, 0x3b84, &[on as u8], seq)
}
/// Zone tracking is a pair of frames that must be sent about 100 ms apart.
pub fn zone_tracking(on: bool, seq1: u16, seq2: u16) -> [[u8; FRAME_SIZE]; 2] {
    [
        frame(TRACKING, 0x08c4, &[on as u8], seq1),
        frame(TRACKING, 0x5084, &[on as u8], seq2),
    ]
}
/// Autofocus mode: face (true) or global (false). Receiver 2, u32 (capture 12).
pub fn autofocus_mode(face: bool, seq: u16) -> [u8; FRAME_SIZE] {
    frame(POWER, 0x3602, &(face as u32).to_le_bytes(), seq)
}
/// Zone tracking: automatically follow new targets.
pub fn zone_follow(on: bool, seq: u16) -> [u8; FRAME_SIZE] {
    frame(TRACKING, 0x4e04, &[on as u8], seq)
}
/// Sleep (true) or wake (false) from the power button.
pub fn sleep(asleep: bool, seq: u16) -> [u8; FRAME_SIZE] {
    frame(POWER, 0xa0c2, &(asleep as u32).to_le_bytes(), seq)
}
/// Factory reset; the camera restarts and deletes its presets.
pub fn factory_reset(seq: u16) -> [u8; FRAME_SIZE] {
    frame(POWER, 0xa802, &[0x01], seq)
}
/// First half of the exposure mode: automatic exposure on (capture 12: \`01 32\`).
pub fn exposure_mode_type(seq: u16) -> [u8; FRAME_SIZE] {
    frame(POWER, 0x2982, &[0x01, 0x32, 0x00, 0x00, 0x00], seq)
}
/// Moves the gimbal with OBSBOT Center's pose command 0x6444. Degrees; yaw positive turns
/// the camera to its left, pitch positive tilts it down; None keeps that axis.
pub fn gimbal_pose(yaw: Option<f32>, pitch: Option<f32>, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    const KEEP: f32 = -1000.0;
    if yaw.is_none() && pitch.is_none()
        || [yaw, pitch].into_iter().flatten().any(|v| !v.is_finite())
    {
        return Err(invalid("gimbal pose needs a finite pan and/or tilt"));
    }
    let mut data = Vec::with_capacity(12);
    for v in [KEEP, pitch.unwrap_or(KEEP), yaw.unwrap_or(KEEP)] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Ok(frame(TRACKING, 0x6444, &data, seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Building with the captured sequence number must reproduce the capture byte for
    /// byte, which proves both CRCs; the rest of the buffer is zero padding.
    fn assert_captured(built: [u8; FRAME_SIZE], captured: &str) {
        let captured = hex::decode(captured).unwrap();
        assert_eq!(built[..captured.len()], captured[..]);
        assert!(built[captured.len()..].iter().all(|b| *b == 0));
    }

    #[test]
    fn sleep_and_wake_match_capture() {
        assert_captured(
            sleep(false, 0xa5),
            "aa25a5000c005fef0a02c2a00400be0700000000",
        );
        assert_captured(
            sleep(true, 0x42),
            "aa2542000c00ea630a02c2a00400bffb01000000",
        );
    }

    #[test]
    fn gestures_match_capture() {
        assert_captured(
            gesture(Gesture::TargetLock, true, 0x4d),
            "aa254d000c00399e0a04c430010027ff01",
        );
        assert_captured(
            gesture(Gesture::TargetLock, false, 0x23),
            "aa2523000c005bd50a04c4300100e63f00",
        );
        assert_captured(
            gesture(Gesture::Zoom, true, 0x4c),
            "aa254c000c00c85b0a044431010027ff01",
        );
        assert_captured(
            gesture(Gesture::DynamicZoom, false, 0x48),
            "aa2548000c00084f0a0444330100e63f00",
        );
        assert_captured(
            gesture(Gesture::DirectionChange, true, 0x49),
            "aa2549000c00384a0a04c433010027ff01",
        );
    }

    #[test]
    fn gesture_zoom_ratio_matches_capture() {
        assert_captured(
            gesture_zoom_ratio(2.0, 0x56).unwrap(),
            "aa2556000c00a9ef0a0444320400bff700000040",
        );
        assert!(gesture_zoom_ratio(4.5, 0).is_err());
    }

    #[test]
    fn tracking_speed_hand_and_invert_match_capture() {
        assert_captured(
            tracking_speed(TrackingSpeed::Standard, 0x20),
            "aa2520000c00abcb0a04c40c0100e63f00",
        );
        assert_captured(
            tracking_speed(TrackingSpeed::Sport, 0x21),
            "aa2521000c00fa0e0a04c40c010067fe02",
        );
        assert_captured(
            hand_side(HandSide::Left, 0x0186),
            "aa2586010c00a11a0a044422010027ff01",
        );
        assert_captured(
            hand_side(HandSide::Right, 0x0187),
            "aa2587010c00f0df0a0444220100e63f00",
        );
        assert_captured(invert(true, 0x5b), "aa255b000c00a8130a04843b010027ff01");
        assert_captured(invert(false, 0x5c), "aa255c000c0019c90a04843b0100e63f00");
    }

    #[test]
    fn zone_tracking_pair_matches_capture() {
        let [a, b] = zone_tracking(true, 0x0194, 0x0195);
        assert_captured(a, "aa2594010c00e15a0a04c408010027ff01");
        assert_captured(b, "aa2595010c0080a50a048450010027ff01");
        let [a, b] = zone_tracking(false, 0x0196, 0x0197);
        assert_captured(a, "aa2596010c0040900a04c4080100e63f00");
        assert_captured(b, "aa2597010c00216f0a0484500100e63f00");
    }

    #[test]
    fn autofocus_mode_matches_capture() {
        // capture 12: global (seq 494), then face (seq 495)
        assert_captured(
            autofocus_mode(false, 494),
            "aa25ee010c00f1200a0202360400be07000000",
        );
        assert_captured(
            autofocus_mode(true, 495),
            "aa25ef010c00a0e50a0202360400bffb01000000",
        );
    }

    #[test]
    fn zone_follow_matches_capture() {
        // capture 11 (seq 452 on, 476 off)
        assert_captured(zone_follow(true, 452), "aa25c4010c0033e90a04044e010027ff01");
        assert_captured(
            zone_follow(false, 476),
            "aa25dc010c00b3960a04044e0100e63f00",
        );
    }

    #[test]
    fn exposure_type_and_factory_reset_match_capture() {
        assert_captured(
            exposure_mode_type(0x15),
            "aa2515000c00a89e0a0282290500f9270132000000",
        );
        assert_eq!(
            exposure_mode_select(ExposureMode::Global),
            [0x03, 0x01, 0x00]
        );
        assert_eq!(exposure_mode_select(ExposureMode::Face), [0x03, 0x01, 0x01]);
        assert_captured(factory_reset(0x0b), "aa250b000c00695e0a0202a8010027ff01");
    }

    #[test]
    fn gimbal_pose_matches_capture() {
        assert_captured(
            gimbal_pose(None, Some(-30.0), 28).unwrap(),
            "aa251c000c000b250a0444640c000f1600007ac40000f0c100007ac4",
        );
        assert_captured(
            gimbal_pose(Some(0.0), Some(0.0), 33).unwrap(),
            "aa2521000c009a200a0444640c0004c400007ac40000000000000000",
        );
        assert_captured(
            gimbal_pose(Some(100.0), None, 87).unwrap(),
            "aa2557000c0078140a0444640c00297e00007ac400007ac40000c842",
        );
        assert!(gimbal_pose(None, None, 0).is_err());
        assert!(gimbal_pose(Some(f32::NAN), None, 0).is_err());
    }

    #[test]
    fn short_commands() {
        assert_eq!(ai_mode(AiMode::UpperBody), [0x16, 0x02, 0x02, 0x01]);
        assert_eq!(ai_mode(AiMode::Hand), [0x16, 0x02, 0x03, 0x00]);
        assert_eq!(auto_sleep(-120), [0x0b, 0x02, 0x88, 0xff]);
        assert_eq!(led(2).unwrap(), [0x1a, 0x01, 0x02]);
        assert!(led(4).is_err());
        assert_eq!(hdr(true), [0x01, 0x01, 0x01]);
    }
}
