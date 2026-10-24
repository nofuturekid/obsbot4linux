// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! Camera presets: write frames and list decoding.
//!
//! Poses use the user convention here: pan degrees positive to the camera's left, tilt
//! degrees positive up. The camera's preset format has tilt positive down, so the sign is
//! flipped when writing and reading.

use crate::error::{Error, Result};
use crate::frame::{FRAME_SIZE, frame};
use serde::{Deserialize, Serialize};

const RECEIVER: u8 = 0x04;
pub const SLOTS: u8 = 3;
pub const MAX_NAME: usize = 32;
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

fn pose_frame(command: u16, slot: u8, pose: Pose, seq: u16) -> Result<[u8; FRAME_SIZE]> {
    check_slot(slot)?;
    if !(pose.pan.is_finite() && pose.tilt.is_finite() && pose.zoom >= 1.0) {
        return Err(Error::Invalid(
            "preset pose needs finite pan/tilt and zoom >= 1".into(),
        ));
    }
    let mut data = (slot as u32).to_le_bytes().to_vec();
    // pan, pitch (down positive), 0, zoom, -1000 as OBSBOT Center sends it
    for v in [pose.pan, -pose.tilt + 0.0, 0.0, pose.zoom, -1000.0] {
        data.extend_from_slice(&v.to_le_bytes());
    }
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
