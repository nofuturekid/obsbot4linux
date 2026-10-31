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
    /// Autofocus mode of the camera (command 0x3442, query 0x3402): `continuous` keeps refocusing
    /// (status byte 0x0e is 1), `single` focuses once and holds, `manual` takes the position of
    /// `focus set`. Measured on a Tiny 2 Lite: 1 sets byte 0x0e, 2 and 3 clear it.
    AutofocusMode { Continuous = 1, Single = 2, Manual = 3 }
);
setting!(
    /// Automatic exposure mode. The camera reports it only as status byte 0x07 (face);
    /// obsbot4linux also remembers the last one set. Manual exposure goes over UVC.
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
/// Auto-sleep off: the current time negated, as OBSBOT Center does.
pub fn auto_sleep_off(current: u16) -> Result<[u8; 4]> {
    // -0 reads back as on, so 0 s goes off as the camera's default 600 s (measured)
    let current = if current == 0 { 600 } else { current };
    let secs = i16::try_from(current).map_err(|_| invalid("auto-sleep time is at most 32767 s"))?;
    Ok(auto_sleep(-secs))
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
/// Status LED blinking (true) or steady (false). Not part of the status block.
pub fn led_blink(on: bool) -> [u8; 3] {
    [0x18, 0x01, on as u8]
}
/// Zoom after waking up, 0-100 (factor 1 + 3 * percent / 100). The camera keeps 100 for anything
/// above (measured), so this refuses it instead.
pub fn wake_zoom(percent: u8) -> Result<[u8; 4]> {
    if percent > 100 {
        return Err(invalid("wake-up zoom is 0-100 percent"));
    }
    Ok([0x15, 0x02, 0x64, percent])
}
/// The wake-up zoom factor (1.0-4.0) as the 0-100 the camera stores.
pub fn wake_zoom_percent(factor: f32) -> Result<u8> {
    if !(1.0..=4.0).contains(&factor) {
        return Err(invalid("wake-up zoom factor is 1.0-4.0"));
    }
    Ok(((factor - 1.0) * 100.0 / 3.0).round() as u8)
}
/// Work mode the camera starts in: main mode in the high nibble, sub-mode in the low one, the
/// same pair as the AI mode (status byte 0x20). The camera ignores values from 0x60 up.
pub fn default_work_mode(mode: AiMode) -> [u8; 3] {
    let [main, sub] = mode.to_bytes();
    [0x19, 0x01, main << 4 | sub]
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
/// Gimbal range in obsbot4linux's angles (pan positive = left, tilt positive = up). Measured on a
/// Tiny 2 Lite: pan stops at about ±148, tilt reaches 90. Below -75 the camera goes to sleep
/// (privacy) and parks at -85; on wake it returns to that pose and sleeps again. -70 is
/// OBSBOT's controllable limit (spec: pan ±140, tilt 30 to -70; mechanical ±150, ±90).
pub const GIMBAL_PAN_LIMIT: f32 = 150.0;
pub const GIMBAL_TILT_MIN: f32 = -70.0;
pub const GIMBAL_TILT_MAX: f32 = 90.0;
/// Whether pan and tilt (obsbot4linux's angles) are within the gimbal range. Rejects NaN, inf and
/// -1000, which means "keep this axis" in 0x6444.
pub fn gimbal_angles_ok(pan: f32, tilt: f32) -> bool {
    pan.abs() <= GIMBAL_PAN_LIMIT && (GIMBAL_TILT_MIN..=GIMBAL_TILT_MAX).contains(&tilt)
}
/// The four area frames (yaw min / max, pitch min / max), sent one after the other as Center does.
pub fn zone_area(
    yaw: (f32, f32),
    pitch: (f32, f32),
    seqs: [u16; 4],
) -> Result<[[u8; FRAME_SIZE]; 4]> {
    // In the camera's own angles: yaw positive = camera left, pitch positive = down (as 0x6444).
    // OBSBOT Center offers yaw -100..100, pitch -30..30 (captures 11, 18); the camera stores
    // more, and zone tracking measured on a Tiny 2 Lite keeps to a tilt of 45 (stopped at 48.7,
    // 90.7 without zone). So the area takes the gimbal range.
    let ok = |(lo, hi): (f32, f32), (min, max): (f32, f32)| {
        lo.is_finite() && hi.is_finite() && lo < hi && lo >= min && hi <= max
    };
    let pitch_range = (-GIMBAL_TILT_MAX, -GIMBAL_TILT_MIN);
    if !ok(yaw, (-GIMBAL_PAN_LIMIT, GIMBAL_PAN_LIMIT)) || !ok(pitch, pitch_range) {
        return Err(invalid(
            "zone area: min must be below max, pan within -150..150, tilt within -70..90",
        ));
    }
    Ok([
        frame(TRACKING, 0x4c04, &yaw.0.to_le_bytes(), seqs[0]),
        frame(TRACKING, 0x4c84, &yaw.1.to_le_bytes(), seqs[1]),
        frame(TRACKING, 0x4d04, &pitch.0.to_le_bytes(), seqs[2]),
        frame(TRACKING, 0x4d84, &pitch.1.to_le_bytes(), seqs[3]),
    ])
}
/// Moves the gimbal to the boot pose (the start pose of the initial state). Does not store
/// anything. Short command `00`: the camera sends the AI request 0x3904 itself. Center's framed
/// 0x3904 does the same but arms the flag that turns the next zone return into Static (#79); the
/// short command does not. The camera ignores it while asleep or with the gimbal locked.
pub fn startup_go() -> [u8; 3] {
    [0x00, 0x01, 0x01]
}
/// Moves the gimbal to the zone return position (`aiTrgLimitedZoneTrackInitPosR`, no data).
pub fn zone_return_go(seq: u16) -> [u8; FRAME_SIZE] {
    frame(TRACKING, 0x4f44, &[], seq)
}
/// Moves the gimbal to the hand-tracking return position (`aiTrgHandTrackInitPosR`, no data).
pub fn hand_return_go(seq: u16) -> [u8; FRAME_SIZE] {
    frame(TRACKING, 0x2584, &[], seq)
}
/// Clears the hand-tracking return position (`aiRstHandTrackInitPosR`, no data).
pub fn hand_return_reset(seq: u16) -> [u8; FRAME_SIZE] {
    frame(TRACKING, 0x2544, &[], seq)
}
/// Autofocus mode: face (true) or global (false). Receiver 2, u32 (capture 12).
pub fn autofocus_mode(face: bool, seq: u16) -> [u8; FRAME_SIZE] {
    frame(POWER, 0x3602, &(face as u32).to_le_bytes(), seq)
}
/// Microphone volume 0-100 (receiver 2, i16). Status byte 0x22 follows: below 50 near, 50-84
/// standard, from 85 far.
pub fn audio_volume(volume: u8, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    if volume > 100 {
        return Err(invalid("microphone volume is 0-100"));
    }
    Ok(frame(POWER, 0x8102, &i16::from(volume).to_le_bytes(), seq))
}
/// Autofocus mode (receiver 2, u32).
pub fn autofocus_mode_select(mode: AutofocusMode, seq: u16) -> [u8; FRAME_SIZE] {
    frame(POWER, 0x3442, &u32::from(mode.to_u8()).to_le_bytes(), seq)
}
/// Focus once on a point of the picture, x and y in 0..1 from the top left (receiver 2): f32 x,
/// f32 y, u32 action 1, then the search bracket towards infinity and macro, both 100 (the
/// camera's own default window). The camera switches to single autofocus.
pub fn focus_point(x: f32, y: f32, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return Err(invalid("focus point x and y are 0.0-1.0"));
    }
    let mut data = Vec::with_capacity(20);
    data.extend_from_slice(&x.to_le_bytes());
    data.extend_from_slice(&y.to_le_bytes());
    for v in [1u32, 100, 100] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Ok(frame(POWER, 0x3582, &data, seq))
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
/// Moves the gimbal with OBSBOT Center's pose command 0x6444. Degrees; yaw positive turns
/// the camera to its left, pitch positive tilts it down; None keeps that axis.
pub fn gimbal_pose(yaw: Option<f32>, pitch: Option<f32>, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    const KEEP: f32 = -1000.0;
    if yaw.is_none() && pitch.is_none()
        || !gimbal_angles_ok(yaw.unwrap_or(0.0), pitch.map_or(0.0, |p| -p))
    {
        return Err(invalid(
            "gimbal pose needs pan -150..150 and/or tilt -70..90",
        ));
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
    fn zone_area_matches_capture() {
        // capture 11, seq 430-433: Center's full area
        let frames = zone_area((-100.0, 100.0), (-30.0, 30.0), [430, 431, 432, 433]).unwrap();
        assert_captured(frames[0], "aa25ae010c0091b60a04044c040068560000c8c2");
        assert_captured(frames[1], "aa25af010c00a1b30a04844c040069f60000c842");
        assert_captured(frames[2], "aa25b0010c0030160a04044d04003b970000f0c1");
        assert_captured(frames[3], "aa25b1010c0000130a04844d04003a370000f041");
    }

    #[test]
    fn hand_return_go_and_reset_have_no_data() {
        for (f, command) in [
            (hand_return_go(9), 0x2584u16),
            (hand_return_reset(9), 0x2544),
        ] {
            assert_eq!(f[0..2], [0xaa, 0x25]);
            assert_eq!(f[10..12], command.to_le_bytes());
            assert_eq!(f[12..14], [0, 0], "data length 0");
            assert!(f[16..].iter().all(|b| *b == 0));
        }
    }

    #[test]
    fn zone_return_go_has_no_data() {
        let f = zone_return_go(9);
        assert_eq!(f[0..2], [0xaa, 0x25]);
        assert_eq!(f[10..12], [0x44, 0x4f]);
        assert_eq!(f[12..14], [0, 0], "data length 0");
        assert!(f[16..].iter().all(|b| *b == 0));
    }

    #[test]
    fn zone_area_rejects_what_the_gimbal_cannot_reach() {
        // an inverted or too wide area would leave the camera without a usable zone; pitch is the
        // camera's (down positive), so tilt -70 is pitch 70: below it the camera goes to sleep
        assert!(zone_area((10.0, -10.0), (-30.0, 30.0), [0; 4]).is_err());
        assert!(zone_area((-150.5, 100.0), (-30.0, 30.0), [0; 4]).is_err());
        assert!(zone_area((-100.0, 100.0), (-30.0, 70.5), [0; 4]).is_err());
        assert!(zone_area((-100.0, 100.0), (-90.5, 30.0), [0; 4]).is_err());
        assert!(zone_area((-150.0, 150.0), (-90.0, 70.0), [0; 4]).is_ok());
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
    fn exposure_and_factory_reset_match_capture() {
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
    fn gimbal_pose_rejects_what_the_camera_would_ignore() {
        // -1000 on the wire keeps the axis, so these would report success without moving it.
        // Pitch is the wire's (down positive): user tilt 90 up is pitch -90, -70 is pitch 70.
        assert!(gimbal_pose(Some(-1000.0), None, 0).is_err());
        assert!(gimbal_pose(Some(1000.0), None, 0).is_err());
        assert!(gimbal_pose(None, Some(-1000.0), 0).is_err());
        assert!(gimbal_pose(None, Some(1000.0), 0).is_err());
        assert!(gimbal_pose(Some(150.5), None, 0).is_err());
        assert!(gimbal_pose(Some(-150.5), None, 0).is_err());
        assert!(gimbal_pose(None, Some(-90.5), 0).is_err());
        assert!(
            gimbal_pose(None, Some(76.0), 0).is_err(),
            "below tilt -75 the camera goes to sleep"
        );
        assert!(gimbal_pose(None, Some(70.5), 0).is_err());
        assert!(gimbal_pose(None, Some(f32::INFINITY), 0).is_err());
        assert!(gimbal_pose(Some(150.0), Some(-90.0), 0).is_ok());
        assert!(gimbal_pose(Some(-150.0), Some(70.0), 0).is_ok());
    }

    #[test]
    fn short_commands() {
        assert_eq!(ai_mode(AiMode::UpperBody), [0x16, 0x02, 0x02, 0x01]);
        assert_eq!(ai_mode(AiMode::Hand), [0x16, 0x02, 0x03, 0x00]);
        assert_eq!(auto_sleep(-120), [0x0b, 0x02, 0x88, 0xff]);
        assert_eq!(auto_sleep_off(120).unwrap(), auto_sleep(-120));
        assert_eq!(led(2).unwrap(), [0x1a, 0x01, 0x02]);
        assert!(led(4).is_err());
        assert_eq!(hdr(true), [0x01, 0x01, 0x01]);
        // a short command, never the framed 0x3904 that arms the zone-Static flag (#79)
        assert_eq!(startup_go(), [0x00, 0x01, 0x01]);
    }

    #[test]
    fn led_blink_and_wake_settings() {
        assert_eq!(led_blink(true), [0x18, 0x01, 0x01]);
        assert_eq!(led_blink(false), [0x18, 0x01, 0x00]);
        assert_eq!(wake_zoom(33).unwrap(), [0x15, 0x02, 0x64, 33]);
        assert!(wake_zoom(101).is_err());
        // 2.0x is the camera's own default (0x21 = 33), 4.0x is the end of the range
        assert_eq!(wake_zoom_percent(2.0).unwrap(), 33);
        assert_eq!(wake_zoom_percent(1.0).unwrap(), 0);
        assert_eq!(wake_zoom_percent(4.0).unwrap(), 100);
        assert!(wake_zoom_percent(4.1).is_err() && wake_zoom_percent(0.9).is_err());
        // main nibble, sub nibble: upper body is mode 2 / sub 1, the camera reads 0x21 back
        assert_eq!(default_work_mode(AiMode::Static), [0x19, 0x01, 0x00]);
        assert_eq!(default_work_mode(AiMode::UpperBody), [0x19, 0x01, 0x21]);
        assert_eq!(default_work_mode(AiMode::Group), [0x19, 0x01, 0x10]);
        assert_eq!(default_work_mode(AiMode::Hand), [0x19, 0x01, 0x30]);
    }

    /// Frames built independently (CRC-16/USB computed in Python) for payloads whose effect was
    /// read back from a Tiny 2 Lite on firmware 6.2.8.1: volume 0x37 read back as 55 ("standard"),
    /// mode 3 as 3, the point (0.3, 0.4) as sent with the camera in single autofocus. OBSBOT Center
    /// sends none of these, so there is no capture to compare with.
    #[test]
    fn audio_and_focus_frames_hold_what_the_camera_read_back() {
        assert_captured(
            audio_volume(0x37, 0x10).unwrap(),
            "aa2510000c00d8f00a0202810200e8363700",
        );
        assert!(audio_volume(101, 0).is_err());
        assert_captured(
            autofocus_mode_select(AutofocusMode::Manual, 0x11),
            "aa2511000c0079420a0242340400be4303000000",
        );
        assert_captured(
            focus_point(0.3, 0.4, 0x12).unwrap(),
            "aa2512000c00188d0a0282351400e5fb9a99993ecdcccc3e010000006400000064000000",
        );
        assert!(focus_point(1.1, 0.5, 0).is_err());
        assert!(focus_point(0.5, f32::NAN, 0).is_err());
    }

    #[test]
    fn auto_sleep_off_is_always_negative() {
        // the status reads >= 0 as on, so -0 would leave auto-sleep on
        assert_eq!(auto_sleep_off(0).unwrap(), auto_sleep(-600));
        // 32768 (status -0x8000) has no positive i16 to negate
        assert_eq!(auto_sleep_off(32767).unwrap(), auto_sleep(-32767));
        assert!(auto_sleep_off(32768).is_err());
    }
}
