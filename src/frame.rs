// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! Framed commands (unit 2, selector 2): CRC-16/USB, query frames, sequence numbers.

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FRAME_SIZE: usize = 60;
const HEADER_SIZE: usize = 12;
const DATA_OFFSET: usize = 16;
/// Largest payload that fits after both headers.
pub const MAX_DATA: usize = FRAME_SIZE - DATA_OFFSET;

/// CRC-16/USB: reflected polynomial 0xA001, init 0xFFFF, final XOR 0xFFFF.
pub fn crc16_usb(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for byte in data {
        crc ^= *byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xffff
}

fn header(kind: u8, receiver: u8, command: u16, sequence: u16) -> [u8; FRAME_SIZE] {
    let mut b = [0u8; FRAME_SIZE];
    b[0..2].copy_from_slice(&[0xaa, kind]);
    b[2..4].copy_from_slice(&sequence.to_le_bytes());
    b[4..6].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    b[8] = 0x0a;
    b[9] = receiver;
    b[10..12].copy_from_slice(&command.to_le_bytes());
    let crc = crc16_usb(&b[0..HEADER_SIZE]);
    b[6..8].copy_from_slice(&crc.to_le_bytes());
    b
}

/// Builds a set frame (type 0x25): header, then data length, data CRC and data.
///
/// # Panics
/// If `data` is longer than [`MAX_DATA`].
pub fn frame(receiver: u8, command: u16, data: &[u8], sequence: u16) -> [u8; FRAME_SIZE] {
    assert!(
        data.len() <= MAX_DATA,
        "frame data must not exceed {MAX_DATA} bytes"
    );
    let mut b = header(0x25, receiver, command, sequence);
    let end = DATA_OFFSET + data.len();
    b[12..14].copy_from_slice(&(data.len() as u16).to_le_bytes());
    b[DATA_OFFSET..end].copy_from_slice(data);
    let crc = crc16_usb(&b[HEADER_SIZE..end]);
    b[14..16].copy_from_slice(&crc.to_le_bytes());
    b
}

/// Builds a query frame (type 0x01): data length 0 and data CRC 0.
pub fn query_frame(receiver: u8, command: u16, sequence: u16) -> [u8; FRAME_SIZE] {
    header(0x01, receiver, command, sequence)
}

/// Answer to a query, matched by sequence number only.
#[derive(Debug, PartialEq)]
pub enum Answer<'a> {
    /// Type 0x29: the value, in the format the setter takes.
    Value(&'a [u8]),
    /// Type 0x89: rejected, with the error code from the data.
    Rejected(u16),
}

/// Parses a selector-2 read; None when it is not the answer to `sequence` (yet).
pub fn parse_answer(reply: &[u8], sequence: u16) -> Option<Answer<'_>> {
    if reply.len() < DATA_OFFSET || reply[0] != 0xaa || reply[2..4] != sequence.to_le_bytes() {
        return None;
    }
    let len = (u16::from_le_bytes([reply[12], reply[13]]) as usize).min(reply.len() - DATA_OFFSET);
    let data = &reply[DATA_OFFSET..DATA_OFFSET + len];
    match reply[1] {
        0x29 => Some(Answer::Value(data)),
        0x89 => Some(Answer::Rejected(u16::from_le_bytes([
            *data.first().unwrap_or(&0),
            *data.get(1).unwrap_or(&0),
        ]))),
        _ => None,
    }
}

/// Sequence numbers for frames. The camera drops a frame that repeats the previous
/// number, so every frame gets a fresh one; seeding from the clock keeps two
/// short-lived processes from starting at the same number.
pub struct Sequence(AtomicU16);

impl Sequence {
    pub fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u16)
            .unwrap_or(0);
        Self(AtomicU16::new(seed))
    }

    pub fn next(&self) -> u16 {
        self.0.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }
}

impl Default for Sequence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_usb_check_value() {
        assert_eq!(crc16_usb(b"123456789"), 0xb4c8);
    }

    #[test]
    fn layout() {
        let f = frame(0x04, 0x3244, &[0x11, 0x22, 0x33], 0x1234);
        assert_eq!(f[0..2], [0xaa, 0x25]);
        assert_eq!(f[2..4], [0x34, 0x12], "sequence little endian");
        assert_eq!(f[10..12], [0x44, 0x32], "full 16 bit command id");
        assert_eq!(f[12..14], [0x03, 0x00]);
        assert_eq!(f[16..19], [0x11, 0x22, 0x33]);
        assert!(f[19..].iter().all(|b| *b == 0));
    }

    #[test]
    fn checksums_cover_header_and_data() {
        let f = frame(0x04, 0x30c4, &[0x01], 7);
        let mut h = f[0..12].to_vec();
        h[6..8].fill(0);
        assert_eq!(f[6..8], crc16_usb(&h).to_le_bytes());
        let mut d = f[12..17].to_vec();
        d[2..4].fill(0);
        assert_eq!(f[14..16], crc16_usb(&d).to_le_bytes());
    }

    #[test]
    #[should_panic]
    fn too_much_data_panics() {
        frame(0x04, 1, &[0u8; MAX_DATA + 1], 0);
    }

    #[test]
    fn query_matches_capture() {
        // Query 0x0104 from OBSBOT Center (capture 01, seq 7).
        let captured = hex::decode("aa0107000c00344e0a04040100000000").unwrap();
        let q = query_frame(0x04, 0x0104, 7);
        assert_eq!(q[..captured.len()], captured[..]);
        assert!(q[captured.len()..].iter().all(|b| *b == 0));
    }

    fn answer(kind: u8, seq: u16, data: &[u8]) -> [u8; 60] {
        let mut r = [0u8; 60];
        r[0] = 0xaa;
        r[1] = kind;
        r[2..4].copy_from_slice(&seq.to_le_bytes());
        r[12..14].copy_from_slice(&(data.len() as u16).to_le_bytes());
        r[16..16 + data.len()].copy_from_slice(data);
        r
    }

    #[test]
    fn answer_is_matched_by_sequence_only() {
        let r = answer(0x29, 7, &[1, 2, 3]);
        assert_eq!(parse_answer(&r, 7), Some(Answer::Value(&[1, 2, 3])));
        assert_eq!(parse_answer(&r, 8), None, "other sequence");
        assert_eq!(parse_answer(&[0u8; 60], 7), None, "not a frame");
    }

    #[test]
    fn rejected_answer_carries_the_code() {
        let r = answer(0x89, 7, &[0x0a, 0x00]);
        assert_eq!(parse_answer(&r, 7), Some(Answer::Rejected(0x000a)));
    }

    #[test]
    fn sequence_never_repeats_consecutively() {
        let s = Sequence::new();
        let mut prev = s.next();
        for _ in 0..70_000 {
            let cur = s.next();
            assert_eq!(cur, prev.wrapping_add(1));
            prev = cur;
        }
    }
}
