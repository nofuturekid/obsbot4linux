// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! Decoding of the status block (unit 2, selector 6) and the 0x0104 tracking readback.
//!
//! Offsets are those of the Tiny 2 Lite, see the protocol reference `status-block.md`.

use crate::commands::{
    AiMode, HandSide, NoiseReduction, PickupDistance, SleepBackground, TrackingSpeed,
};
use serde::Serialize;

/// Field of view as the camera reports it. The presets are digital zoom levels (wide 1.0,
/// medium 1.15, narrow 1.47 by query 0x6804): setting one sets the zoom, and setting the zoom
/// to any other factor turns status byte 0x11 to 3, reported as `custom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FovReading {
    Wide,
    Medium,
    Narrow,
    Custom,
}

impl FovReading {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Wide),
            1 => Some(Self::Medium),
            2 => Some(Self::Narrow),
            3 => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub asleep: bool,
    /// None for a combination the reference does not list (also while switching modes).
    pub ai_mode: Option<AiMode>,
    pub tracking_speed: TrackingSpeed,
    pub hdr: bool,
    pub auto_sleep_enabled: bool,
    pub auto_sleep_seconds: u16,
    /// Own image (3) is not settable from obsbot4linux; reported as None.
    pub sleep_background: Option<SleepBackground>,
    pub sleep_background_mirror: bool,
    /// Status byte 0x11; `custom` (3) once the zoom was set to a factor other than a preset.
    pub field_of_view: Option<FovReading>,
    pub mirror: bool,
    pub led: u8,
    pub mic_in_sleep: bool,
    pub noise_reduction: Option<NoiseReduction>,
    pub auto_gain: bool,
    pub pickup_distance: Option<PickupDistance>,
    pub gesture_target_lock: bool,
    /// V4L2 power_line_frequency: 0 off, 1 50 Hz, 2 60 Hz.
    pub anti_flicker: u8,
    pub digital_zoom_percent: u8,
    pub face_auto_exposure: bool,
    pub autofocus_face: bool,
    pub continuous_autofocus: bool,
    pub manual_focus: u8,
    pub frame_rate: u8,
}

impl Status {
    pub fn decode(b: &[u8; 60]) -> Self {
        let auto_sleep = i16::from_le_bytes([b[0x0a], b[0x0b]]);
        Self {
            asleep: b[0x02] == 1,
            ai_mode: AiMode::from_status(b[0x18], b[0x1c]),
            tracking_speed: TrackingSpeed::from_u8(b[0x24]).unwrap_or(TrackingSpeed::Standard),
            hdr: b[0x06] != 0,
            auto_sleep_enabled: auto_sleep >= 0,
            auto_sleep_seconds: auto_sleep.unsigned_abs(),
            sleep_background: SleepBackground::from_u8(b[0x1b]),
            sleep_background_mirror: b[0x1d] != 0,
            field_of_view: FovReading::from_u8(b[0x11]),
            mirror: b[0x13] != 0,
            led: b[0x21],
            mic_in_sleep: b[0x10] != 0,
            noise_reduction: NoiseReduction::from_u8(b[0x08]),
            auto_gain: b[0x19] != 0,
            pickup_distance: PickupDistance::from_u8(b[0x22] & 0x0f),
            gesture_target_lock: b[0x01] != 0,
            anti_flicker: b[0x03],
            digital_zoom_percent: b[0x04],
            face_auto_exposure: b[0x07] != 0,
            autofocus_face: b[0x0d] != 0,
            continuous_autofocus: b[0x0e] != 0,
            manual_focus: b[0x0f],
            frame_rate: b[0x1f],
        }
    }
}

/// The 12-byte answer to query 0x0104.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tracking {
    pub ai_mode: Option<AiMode>,
    pub hand_side: HandSide,
    pub target_lock: bool,
    pub zoom_gesture: bool,
    pub dynamic_zoom: bool,
    pub direction_change: bool,
    /// Gesture zoom ratio, 1.0-4.0.
    pub gesture_zoom_ratio: f32,
    pub zone_tracking: bool,
    pub invert: bool,
}

impl Tracking {
    pub fn decode(d: &[u8]) -> Option<Self> {
        if d.len() < 12 {
            return None;
        }
        let ai_mode = match (d[0], d[1]) {
            (0, _) => Some(AiMode::Static),
            (1, _) => Some(AiMode::Group),
            (3, _) => Some(AiMode::Hand),
            (m, n) => AiMode::from_status(m, n),
        };
        Some(Self {
            ai_mode,
            hand_side: if d[2] == 1 {
                HandSide::Left
            } else {
                HandSide::Right
            },
            target_lock: d[3] != 0,
            zoom_gesture: d[4] != 0,
            dynamic_zoom: d[5] != 0,
            direction_change: d[6] != 0,
            gesture_zoom_ratio: d[7] as f32 / 10.0,
            // byte 8 (speed) did not follow the setting live; the speed is status byte 0x24.
            // byte 9 is always 0; zone tracking is byte 10
            zone_tracking: d[10] != 0,
            invert: d[11] != 0,
        })
    }
}

/// The tracking target from query 0x01c4 while AI tracking is on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Target {
    /// True while the camera has a target (status LED blue), false while it searches (yellow).
    pub detected: bool,
    /// Left, top, right, bottom, normalised (0..1) to the picture; the last box while searching.
    #[serde(rename = "box")]
    pub coords: [f32; 4],
}

impl Target {
    /// Decodes the answer to query 0x01c4: `01 01 <detected> 01 00 00` and four f32 (LE). The
    /// one-byte `00` of Static and anything short or garbled give None.
    pub fn decode(d: &[u8]) -> Option<Self> {
        if d.len() < 22 || d[0] != 1 || d[2] > 1 {
            return None;
        }
        let mut coords = [0f32; 4];
        for (i, c) in coords.iter_mut().enumerate() {
            let at = 6 + 4 * i;
            *c = f32::from_le_bytes(d[at..at + 4].try_into().ok()?);
            if !c.is_finite() {
                return None;
            }
        }
        Some(Self {
            detected: d[2] == 1,
            coords,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(set: &[(usize, u8)]) -> [u8; 60] {
        let mut b = [0u8; 60];
        for (i, v) in set {
            b[*i] = *v;
        }
        b
    }

    #[test]
    fn lite_offsets() {
        // LED at 0x21 and speed at 0x24 (tiny4linux 2.2.1 read the speed from 0x21),
        // auto-sleep negative = off with the time kept, pickup distance in the low bits.
        let b = block(&[
            (0x21, 3),
            (0x24, 2),
            (0x0a, 0x88),
            (0x0b, 0xff),
            (0x22, 0x12),
            (0x02, 1),
        ]);
        let s = Status::decode(&b);
        assert_eq!(s.led, 3);
        assert_eq!(s.tracking_speed, TrackingSpeed::Sport);
        assert!(!s.auto_sleep_enabled);
        assert_eq!(s.auto_sleep_seconds, 120);
        assert_eq!(s.pickup_distance, Some(PickupDistance::Far));
        assert!(s.asleep);
    }

    #[test]
    fn field_of_view_custom_zoom() {
        // A zoom factor other than a preset level turns byte 0x11 to 3 (read live)
        let fov = |v| Status::decode(&block(&[(0x11, v)])).field_of_view;
        assert_eq!(fov(1), Some(FovReading::Medium));
        assert_eq!(fov(3), Some(FovReading::Custom));
    }

    #[test]
    fn ai_mode_pairs() {
        let mode = |m, n| Status::decode(&block(&[(0x18, m), (0x1c, n)])).ai_mode;
        assert_eq!(mode(0, 0), Some(AiMode::Static));
        assert_eq!(mode(2, 1), Some(AiMode::UpperBody));
        assert_eq!(mode(3, 0), Some(AiMode::Hand));
        assert_eq!(mode(6, 0), None, "switching transient is not hand");
    }

    #[test]
    fn tracking_readback_from_live_camera() {
        // Zone tracking off/on as read live: byte 10, not byte 9.
        let off = hex::decode("000000010101001400000000").unwrap();
        let on = hex::decode("000000010101001400000100").unwrap();
        assert!(!Tracking::decode(&off).unwrap().zone_tracking);
        let t = Tracking::decode(&on).unwrap();
        assert!(t.zone_tracking);
        assert_eq!(t.gesture_zoom_ratio, 2.0);
        assert_eq!(t.ai_mode, Some(AiMode::Static));
    }

    #[test]
    fn tracking_fields() {
        let t = Tracking::decode(&[3, 0, 1, 1, 0, 1, 0, 0x23, 2, 0, 1, 1]).unwrap();
        assert_eq!(t.ai_mode, Some(AiMode::Hand));
        assert_eq!(t.hand_side, HandSide::Left);
        assert!(t.target_lock && t.dynamic_zoom && t.zone_tracking && t.invert);
        assert!(!t.zoom_gesture && !t.direction_change);
        assert_eq!(t.gesture_zoom_ratio, 3.5);
        assert!(Tracking::decode(&[0; 11]).is_none());
    }

    #[test]
    fn target_from_live_camera() {
        // Real replies, firmware 6.2.8.1: blue LED, yellow LED, Static.
        let blue = hex::decode("0101010100005949323e6ca63a3e046d2d3f0000803f").unwrap();
        let t = Target::decode(&blue).unwrap();
        assert!(t.detected);
        assert!((t.coords[0] - 0.174).abs() < 1e-3 && (t.coords[3] - 1.0).abs() < 1e-6);
        let yellow = hex::decode("010100010000e771c43eb997ff3dba56373fce473c3f").unwrap();
        let t = Target::decode(&yellow).unwrap();
        assert!(!t.detected);
        assert!((t.coords[0] - 0.384).abs() < 1e-3);
        assert_eq!(Target::decode(&[0]), None);
    }

    #[test]
    fn target_garbled_replies_are_none() {
        let blue = hex::decode("0101010100005949323e6ca63a3e046d2d3f0000803f").unwrap();
        assert_eq!(Target::decode(&[]), None);
        assert_eq!(Target::decode(&blue[..21]), None);
        let mut bad_flag = blue.clone();
        bad_flag[2] = 7;
        assert_eq!(Target::decode(&bad_flag), None);
        let mut nan = blue.clone();
        nan[6..10].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(Target::decode(&nan), None);
    }
}
