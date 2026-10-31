// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! Camera presets: write frames and list decoding.
//!
//! Poses use the user convention here: pan degrees positive to the camera's left, tilt
//! degrees positive up. The camera's preset format has tilt positive down, so the sign is
//! flipped when writing and reading.

use crate::commands::{TrackingSpeed, gimbal_angles_ok};
use crate::error::{Error, Result};
use crate::frame::{FRAME_SIZE, frame};
use serde::{Deserialize, Serialize};

const RECEIVER: u8 = 0x04;
pub const SLOTS: u8 = 3;
pub const MAX_NAME: usize = 32;
pub const MAX_ZOOM: f32 = 4.0;
const NAME_OFFSET: usize = 10;

/// Per-preset picture settings OBSBOT Center writes after a delete: everything "not set".
const PICTURE_SETTINGS_CLEARED: [u8; 44] = [
    0x00, 0x00, 0x00, 0x00, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    /// Degrees, positive turns the camera to its left.
    pub pan: f32,
    /// Degrees, positive tilts up.
    pub tilt: f32,
    /// Zoom factor, 1.0 = not zoomed.
    pub zoom: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preset {
    /// Slot index 0-2.
    pub slot: u8,
    pub name: String,
    pub pose: Pose,
}

pub fn check_slot(slot: u8) -> Result<()> {
    if slot < SLOTS {
        Ok(())
    } else {
        Err(Error::Invalid(format!("preset slot is 0-{}", SLOTS - 1)))
    }
}

/// The pose layout of presets, start pose and return positions: `lead`, then pan, pitch (down
/// positive), 0, zoom and `last`. Rejects poses outside the gimbal range or zoom 1.0-4.0; the
/// preset readback holds zoom as u16 hundredths and the angles as i16 hundredths.
fn pose_data(lead: u32, pose: Pose, last: f32, what: &str) -> Result<Vec<u8>> {
    if !(gimbal_angles_ok(pose.pan, pose.tilt) && (1.0..=MAX_ZOOM).contains(&pose.zoom)) {
        return Err(Error::Invalid(format!(
            "{what} needs pan -150..150, tilt -70..90 and zoom 1.0-4.0"
        )));
    }
    let mut data = lead.to_le_bytes().to_vec();
    for v in [pose.pan, -pose.tilt + 0.0, 0.0, pose.zoom, last] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Ok(data)
}

fn pose_frame(command: u16, slot: u8, pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    check_slot(slot)?;
    // last value -1000 as OBSBOT Center sends it
    let data = pose_data(slot as u32, pose, -1000.0, "preset pose")?;
    Ok(frame(RECEIVER, command, &data, seq))
}

pub fn create(slot: u8, pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    pose_frame(0x3944, slot, pose, seq)
}

pub fn update(slot: u8, pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    pose_frame(0x3e04, slot, pose, seq)
}

/// Rename: plain ASCII, no terminator.
pub fn rename(slot: u8, name: &str, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    check_slot(slot)?;
    if name.is_empty()
        || name.len() > MAX_NAME
        || !name.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
    {
        return Err(Error::Invalid(format!(
            "preset name must be 1-{MAX_NAME} printable ASCII characters"
        )));
    }
    let mut data = (slot as u32).to_le_bytes().to_vec();
    data.extend_from_slice(name.as_bytes());
    Ok(frame(RECEIVER, 0x3a84, &data, seq))
}

/// Delete is two frames, the delete and the cleared picture settings, about 100 ms apart.
pub fn delete(slot: u8, seq1: u16, seq2: u16) -> Result<[[u8; FRAME_SIZE]; 2]> {
    check_slot(slot)?;
    Ok([
        frame(RECEIVER, 0x3984, &(slot as u32).to_le_bytes(), seq1),
        frame(RECEIVER, 0x3d84, &PICTURE_SETTINGS_CLEARED, seq2),
    ])
}

/// Recall: the slot, then four f32 1.0 as the vendor software sends them.
pub fn recall(slot: u8, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    check_slot(slot)?;
    let mut data = [0u8; 20];
    data[0..4].copy_from_slice(&(slot as u32).to_le_bytes());
    for i in 1..5 {
        data[i * 4..(i + 1) * 4].copy_from_slice(&1.0f32.to_le_bytes());
    }
    Ok(frame(RECEIVER, 0x39c4, &data, seq))
}

/// Start pose (initial state, applied at power-on): `source` 3 = own pose, 0-2 = preset slot.
/// Degrees and zoom factor as for presets (user tilt, positive up, flipped on the wire).
pub fn startup_pose(source: u32, pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    let data = pose_data(source, pose, 0.0, "start pose")?;
    Ok(frame(RECEIVER, 0x3ec4, &data, seq))
}

/// Zone return position (`0x4e84`): index 0, then the preset layout with a 0 in place of the
/// last value. Degrees and zoom factor as for presets (tilt positive up here, flipped on the wire).
pub fn zone_return_set(pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    return_set(0x4e84, pose, seq)
}

/// Hand-tracking return position (`0x24c4`): the same layout as [`zone_return_set`].
pub fn hand_return_set(pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    return_set(0x24c4, pose, seq)
}

fn return_set(command: u16, pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    let data = pose_data(0, pose, 0.0, "return position")?;
    Ok(frame(0x04, command, &data, seq))
}

/// Decodes the 24-byte pose of the readbacks `0x4ec4` (zone return position) and `0x3884`
/// (start pose): the leading `u32` and the pose in user angles. None when the reply is short or
/// holds a value that is not a number.
pub fn parse_pose(data: &[u8]) -> Option<(u32, Pose)> {
    let word = |i: usize| -> Option<[u8; 4]> { data.get(i * 4..i * 4 + 4)?.try_into().ok() };
    let float = |i: usize| word(i).map(f32::from_le_bytes).filter(|v| v.is_finite());
    Some((
        u32::from_le_bytes(word(0)?),
        Pose {
            pan: float(1)?,
            tilt: -float(2)? + 0.0,
            zoom: float(4)?,
        },
    ))
}

/// Picture settings of the initial state with every group "not set", as in capture 08.
pub fn startup_picture_settings(seq: u16) -> [u8; FRAME_SIZE] {
    let mut data = [0u8; 40];
    data[0..8].copy_from_slice(&[0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    data[8..16].copy_from_slice(&[0x80, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    data[20..24].copy_from_slice(&[0xff; 4]);
    frame(RECEIVER, 0x3e44, &data, seq)
}

/// The picture settings of the initial state, from the 40-byte answer to `0x3e84` (the layout of
/// `0x3e44`). A group the initial state does not set is `None`: its first byte is `0xff`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StartupActions {
    pub tracking_speed: Option<TrackingSpeed>,
    pub autofocus: Option<bool>,
    pub auto_exposure: Option<bool>,
    /// Manual white balance in Kelvin.
    pub white_balance_kelvin: Option<u16>,
    pub contrast: Option<u8>,
    pub saturation: Option<u8>,
    pub sharpness: Option<u8>,
    pub hue: Option<u8>,
}

impl StartupActions {
    pub fn decode(d: &[u8]) -> Option<Self> {
        if d.len() < 24 {
            return None;
        }
        let flag = |i: usize| (d[i] != 0xff).then_some(d[i] == 1);
        let level = |i: usize| (d[i] <= 100).then_some(d[i]);
        Some(Self {
            tracking_speed: TrackingSpeed::from_u8(d[1]),
            autofocus: flag(2),
            auto_exposure: flag(6),
            white_balance_kelvin: (d[13] == 5).then(|| u16::from_le_bytes([d[14], d[15]])),
            contrast: level(20),
            saturation: level(21),
            sharpness: level(22),
            hue: level(23),
        })
    }
}

/// Occupied slots from a selector-0x0c read: count, then the slot indices.
pub fn parse_slots(block: &[u8]) -> Vec<u8> {
    let count = block.first().copied().unwrap_or(0) as usize;
    block
        .iter()
        .skip(1)
        .take(count.min(SLOTS as usize))
        .copied()
        .filter(|s| *s < SLOTS)
        .collect()
}

/// One selector-0x0d read: status, slot, pad, pitch i16, yaw i16, zoom u16 (hundredths),
/// then the name as NUL-terminated base64. None at the end of the list.
pub fn parse_entry(block: &[u8]) -> Option<Preset> {
    if block.len() <= NAME_OFFSET || block[0] != 0 || block[1] >= SLOTS {
        return None;
    }
    let hundredths = |o: usize| i16::from_le_bytes([block[o], block[o + 1]]) as f32 / 100.0;
    let name = block[NAME_OFFSET..]
        .split(|b| *b == 0)
        .next()
        .and_then(decode_base64)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    Some(Preset {
        slot: block[1],
        name,
        pose: Pose {
            pan: hundredths(6),
            tilt: -hundredths(4) + 0.0,
            zoom: u16::from_le_bytes([block[8], block[9]]) as f32 / 100.0,
        },
    })
}

fn decode_base64(text: &[u8]) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut bytes = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for c in text.iter().copied().filter(|c| *c != b'=') {
        buffer = (buffer << 6) | value(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_captured(built: [u8; FRAME_SIZE], captured: &str) {
        let captured = hex::decode(captured).unwrap();
        assert_eq!(built[..captured.len()], captured[..]);
        assert!(built[captured.len()..].iter().all(|b| *b == 0));
    }

    // Frames from OBSBOT Center (capture 10). Preset tilt is positive down, so the
    // captured tilt -0.1 is user tilt +0.1.
    #[test]
    fn startup_actions_nothing_set_as_read_live() {
        // `0x3e84` on a camera whose initial state sets nothing (firmware 6.2.8.1): the same
        // bytes `startup_picture_settings` writes. Every group is "not set", not zero/off.
        let d = hex::decode(
            "feffffffffffffff80ffffffffffffff00000000ffffffff00000000000000000000000000000000",
        )
        .unwrap();
        let a = StartupActions::decode(&d).unwrap();
        assert_eq!(a.tracking_speed, None);
        assert_eq!(a.autofocus, None);
        assert_eq!(a.auto_exposure, None);
        assert_eq!(a.white_balance_kelvin, None);
        assert_eq!(a.contrast, None);
    }

    #[test]
    fn startup_actions_set_groups() {
        // Layout from capture 08b: contrast 60 is 0x3c, manual white balance is 05 then Kelvin
        // (`d8 0e` = 3800), autofocus and exposure 01 when set. Built by hand, not read back.
        let mut d = [0xffu8; 40];
        d[1] = 2;
        d[2] = 1;
        d[6] = 0;
        d[13] = 5;
        d[14..16].copy_from_slice(&[0xd8, 0x0e]);
        d[20] = 60;
        let a = StartupActions::decode(&d).unwrap();
        assert_eq!(a.tracking_speed, Some(TrackingSpeed::Sport));
        assert_eq!(a.autofocus, Some(true));
        assert_eq!(a.auto_exposure, Some(false));
        assert_eq!(a.white_balance_kelvin, Some(3800));
        assert_eq!(a.contrast, Some(60));
        assert_eq!(a.saturation, None, "0xff is not set, not 255");
        assert_eq!(StartupActions::decode(&d[..23]), None);
    }

    #[test]
    fn create_and_update_match_capture() {
        let pose = Pose {
            pan: 0.8,
            tilt: 0.1,
            zoom: 1.0,
        };
        assert_captured(
            create(0, pose, 265).unwrap(),
            "aa2509010c00d6c90a04443918009af500000000cdcc4c3fcdccccbd000000000000803f00007ac4",
        );
        let pose = Pose {
            pan: f32::from_bits(0x3f666667),
            tilt: 0.2,
            zoom: 1.0,
        };
        assert_captured(
            update(0, pose, 270).unwrap(),
            "aa250e010c0017110a04043e18000fb4000000006766663fcdcc4cbe000000000000803f00007ac4",
        );
    }

    #[test]
    fn rename_matches_capture() {
        assert_captured(
            rename(0, "Voreingestellt1", 266).unwrap(),
            "aa250a010c0036c70a04843a1300645300000000566f7265696e67657374656c6c7431",
        );
        assert_captured(
            rename(0, "eins", 267).unwrap(),
            "aa250b010c0067020a04843a0800d9770000000065696e73",
        );
    }

    #[test]
    fn delete_matches_capture() {
        let [d, c] = delete(1, 362, 363).unwrap();
        assert_captured(d, "aa256a010c0075780a0484390400bffb01000000");
        assert_captured(
            c,
            "aa256b010c00257e0a04843d2c002a5f00000000feffffffffffffff80ffffffffffffff00000000ffffffff00ffffffffffffffff00000000000000",
        );
    }

    #[test]
    fn zone_return_set_matches_the_preset_layout() {
        // pan 20, tilt 5 up (user angles, so -5 on the wire), zoom 1, as measured
        let f = zone_return_set(
            Pose {
                pan: 20.0,
                tilt: 5.0,
                zoom: 1.0,
            },
            7,
        )
        .unwrap();
        assert_eq!(f[10..12], [0x84, 0x4e]);
        assert_eq!(
            f[16..40],
            hex::decode("000000000000a0410000a0c0000000000000803f00000000").unwrap()[..]
        );
        assert!(
            zone_return_set(
                Pose {
                    pan: 0.0,
                    tilt: 0.0,
                    zoom: 0.5
                },
                0
            )
            .is_err()
        );
    }

    #[test]
    fn hand_return_set_uses_the_zone_layout_with_its_own_command() {
        // measured: pan 20.1, tilt 4.9 up, zoom 1 read back from 0x2504 as sent
        let pose = Pose {
            pan: 20.0,
            tilt: 5.0,
            zoom: 1.0,
        };
        let hand = hand_return_set(pose, 7).unwrap();
        let zone = zone_return_set(pose, 7).unwrap();
        assert_eq!(hand[10..12], [0xc4, 0x24]);
        assert_eq!(hand[16..40], zone[16..40]);
        assert!(hand_return_set(Pose { zoom: 5.0, ..pose }, 0).is_err());
    }

    #[test]
    fn unset_hand_return_reads_as_minus_one() {
        // 0x2504 while none is set (capture 11): leading -1, pan 0, tilt 0, zoom 1
        let (lead, p) =
            parse_pose(&hex::decode("ffffffff0000000000000000000000000000803f00000000").unwrap())
                .unwrap();
        assert_eq!(lead, u32::MAX);
        assert_eq!((p.pan, p.tilt, p.zoom), (0.0, 0.0, 1.0));
    }

    #[test]
    fn pose_readbacks_decode_as_measured() {
        // 0x4ec4 reply from the camera: pan 20, wire tilt -5 (= 5 up), zoom 1
        let (lead, p) =
            parse_pose(&hex::decode("000000000000a0410000a0c0000000000000803f00000000").unwrap())
                .unwrap();
        assert_eq!(lead, 0);
        assert_eq!((p.pan, p.tilt, p.zoom), (20.0, 5.0, 1.0));
        // 0x3884 reply: leading 3 = own pose, pan 10
        let (lead, p) =
            parse_pose(&hex::decode("030000000000204100000000000000000000803f00000000").unwrap())
                .unwrap();
        assert_eq!(lead, 3);
        assert_eq!((p.pan, p.tilt, p.zoom), (10.0, 0.0, 1.0));
        assert!(!p.tilt.is_sign_negative(), "no -0 in the output");
        assert!(parse_pose(&[0; 12]).is_none());
        assert!(parse_pose(&[0xff; 24]).is_none(), "NaN is no pose");
    }

    #[test]
    fn startup_matches_capture() {
        // capture 08 (own pose: pan -3, wire pitch -17.1) and capture 10 (preset 0's pose)
        let pose = Pose {
            pan: -3.0,
            tilt: 17.1,
            zoom: 1.0,
        };
        assert_captured(
            startup_pose(3, pose, 129).unwrap(),
            "aa2581000c007d590a04c43e1800179f03000000000040c0cdcc88c1000000000000803f",
        );
        let pose = Pose {
            pan: f32::from_bits(0x3f666667),
            tilt: 0.2,
            zoom: 1.0,
        };
        assert_captured(
            startup_pose(0, pose, 359).unwrap(),
            "aa2567010c0094800a04c43e18002d47000000006766663fcdcc4cbe000000000000803f",
        );
        assert!(startup_pose(3, Pose { zoom: 5.0, ..pose }, 0).is_err());
        assert_captured(
            startup_picture_settings(130),
            "aa2582000c00ec960a04443e28004dd4feffffffffffffff80ffffffffffffff00000000ffffffff",
        );
    }

    #[test]
    fn recall_matches_capture() {
        assert_captured(
            recall(0, 0x20).unwrap(),
            "aa2520000c006bdc0a04c4391400d6fb000000000000803f0000803f0000803f0000803f",
        );
        assert_captured(
            recall(2, 0x26).unwrap(),
            "aa2526000c008bc30a04c4391400af19020000000000803f0000803f0000803f0000803f",
        );
    }

    #[test]
    fn invalid_input_is_rejected() {
        let pose = Pose {
            pan: 0.0,
            tilt: 0.0,
            zoom: 1.0,
        };
        assert!(create(3, pose, 0).is_err());
        assert!(recall(3, 0).is_err());
        assert!(delete(3, 0, 1).is_err());
        assert!(rename(0, "", 0).is_err());
        assert!(rename(0, "Büro", 0).is_err());
        assert!(create(0, Pose { zoom: 0.5, ..pose }, 0).is_err());
    }

    #[test]
    fn poses_outside_the_gimbal_range_are_rejected() {
        // inf zoom would go out as 0x7f800000 and could not be read back as u16 hundredths;
        // -1000 is "keep this axis" for the gimbal; 400 would wrap in the i16 readback
        let bad = [
            (0.0, 0.0, f32::INFINITY),
            (0.0, 0.0, 1e9),
            (0.0, 0.0, 4.01),
            (0.0, 0.0, f32::NAN),
            (-1000.0, 0.0, 1.0),
            (1000.0, 0.0, 1.0),
            (150.5, 0.0, 1.0),
            (400.0, 0.0, 1.0),
            (0.0, 90.5, 1.0),
            (0.0, -70.5, 1.0),
            (0.0, -76.0, 1.0),
            (0.0, -1000.0, 1.0),
        ];
        for (pan, tilt, zoom) in bad {
            let p = Pose { pan, tilt, zoom };
            assert!(create(0, p, 0).is_err(), "create {p:?}");
            assert!(update(0, p, 0).is_err(), "update {p:?}");
            assert!(startup_pose(3, p, 0).is_err(), "startup {p:?}");
            assert!(zone_return_set(p, 0).is_err(), "zone return {p:?}");
            assert!(hand_return_set(p, 0).is_err(), "hand return {p:?}");
        }
        for (pan, tilt) in [(-150.0, 90.0), (150.0, -70.0)] {
            let edge = Pose {
                pan,
                tilt,
                zoom: MAX_ZOOM,
            };
            assert!(create(0, edge, 0).is_ok(), "create {edge:?}");
            assert!(startup_pose(3, edge, 0).is_ok(), "startup {edge:?}");
            assert!(zone_return_set(edge, 0).is_ok(), "zone return {edge:?}");
        }
    }

    // Read from a Tiny 2 Lite after writing pan 12.5, tilt -3.25 (camera convention), zoom 1.5.
    #[test]
    fn entry_read_from_the_camera() {
        let mut block = [0u8; 60];
        let read = hex::decode("00000000bbfee2049600644452736447567a64413d3d").unwrap();
        block[..read.len()].copy_from_slice(&read);
        let p = parse_entry(&block).unwrap();
        assert_eq!(p.slot, 0);
        assert_eq!(p.name, "t4ltest");
        assert_eq!(
            p.pose,
            Pose {
                pan: 12.5,
                tilt: 3.25,
                zoom: 1.5
            }
        );
        block[0] = 0x02;
        assert_eq!(parse_entry(&block), None, "end of list");
    }

    #[test]
    fn slot_list() {
        assert_eq!(parse_slots(&[0x00]), Vec::<u8>::new());
        assert_eq!(parse_slots(&[0x02, 0x00, 0x02]), vec![0, 2]);
        assert_eq!(parse_slots(&[0x02, 0x00, 0x07]), vec![0]);
    }
}
