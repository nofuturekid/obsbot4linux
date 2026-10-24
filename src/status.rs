// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! Decoding of the status block (unit 2, selector 6) and the 0x0104 tracking readback.
//!
//! Offsets are those of the Tiny 2 Lite, see the protocol reference `status-block.md`.

use crate::commands::{
    AiMode, FieldOfView, HandSide, NoiseReduction, PickupDistance, SleepBackground, TrackingSpeed,
};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub asleep: bool,
    /// None for a combination the reference does not list (also while switching modes).
    pub ai_mode: Option<AiMode>,
    pub tracking_speed: TrackingSpeed,
    pub hdr: bool,
    pub auto_sleep_enabled: bool,
    pub auto_sleep_seconds: u16,
    /// Own image (3) is not settable from t2l; reported as None.
    pub sleep_background: Option<SleepBackground>,
    pub sleep_background_mirror: bool,
    /// None while a digital zoom through V4L2 shows 3.
    pub field_of_view: Option<FieldOfView>,
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
            field_of_view: FieldOfView::from_u8(b[0x11]),
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
            hand_side: if d[2] == 1 { HandSide::Left } else { HandSide::Right },
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
        let b = block(&[(0x21, 3), (0x24, 2), (0x0a, 0x88), (0x0b, 0xff), (0x22, 0x12), (0x02, 1)]);
        let s = Status::decode(&b);
        assert_eq!(s.led, 3);
        assert_eq!(s.tracking_speed, TrackingSpeed::Sport);
        assert!(!s.auto_sleep_enabled);
        assert_eq!(s.auto_sleep_seconds, 120);
        assert_eq!(s.pickup_distance, Some(PickupDistance::Far));
        assert!(s.asleep);
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
}
