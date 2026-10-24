// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Tiny4Linux (EUPL-1.2), used under EUPL-1.2 Article 5.

//! Device lookup and ioctls: UVC extension-unit queries and V4L2 controls.
//!
//! The device is opened per operation and closed on drop, so streaming applications keep
//! working: control ioctls do not need exclusive access.

use crate::error::{Error, Result};
use nix::{ioctl_read, ioctl_readwrite};
use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::PathBuf;

const UVC_SET_CUR: u8 = 0x01;
const UVC_GET_CUR: u8 = 0x81;
const UVC_GET_LEN: u8 = 0x85;

/// Size of every vendor buffer on unit 2.
pub const XU_SIZE: usize = 60;

const V4L2_CAP_META_CAPTURE: u32 = 0x0080_0000;
const V4L2_CTRL_FLAG_DISABLED: u32 = 0x0001;

#[repr(C)]
struct UvcXuControlQuery {
    unit: u8,
    selector: u8,
    query: u8,
    size: u16,
    data: *mut u8,
}

#[repr(C)]
#[derive(Default)]
struct V4l2Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Default)]
struct V4l2Control {
    id: u32,
    value: i32,
}

#[repr(C)]
#[derive(Default)]
struct V4l2QueryCtrl {
    id: u32,
    kind: u32,
    name: [u8; 32],
    minimum: i32,
    maximum: i32,
    step: i32,
    default_value: i32,
    flags: u32,
    reserved: [u32; 2],
}

ioctl_readwrite!(uvcioc_ctrl_query, b'u', 0x21, UvcXuControlQuery);
ioctl_read!(vidioc_querycap, b'V', 0, V4l2Capability);
ioctl_readwrite!(vidioc_g_ctrl, b'V', 27, V4l2Control);
ioctl_readwrite!(vidioc_s_ctrl, b'V', 28, V4l2Control);
ioctl_readwrite!(vidioc_queryctrl, b'V', 36, V4l2QueryCtrl);

/// Range and default of a V4L2 control.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct CtrlRange {
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub default: i32,
}

pub struct Device(File);

fn card_name(file: &File) -> Option<(String, u32)> {
    let mut cap = V4l2Capability::default();
    unsafe { vidioc_querycap(file.as_raw_fd(), &mut cap) }.ok()?;
    let end = cap
        .card
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(cap.card.len());
    Some((
        String::from_utf8_lossy(&cap.card[..end]).into_owned(),
        cap.device_caps,
    ))
}

impl Device {
    /// Finds the Tiny 2 Lite: by-id symlink first, V4L2 card name as fallback.
    pub fn open() -> Result<Self> {
        let by_id = fs::read_dir("/dev/v4l/by-id")
            .into_iter()
            .flatten()
            .flatten();
        for entry in by_id {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains("OBSBOT_Tiny_2_Lite")
                && name.ends_with("-video-index0")
                && let Ok(file) = File::open(entry.path())
            {
                return Ok(Self(file));
            }
        }

        let mut paths: Vec<PathBuf> = fs::read_dir("/dev")?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("video"))
            })
            .collect();
        paths.sort();
        for path in paths {
            if let Ok(file) = File::open(&path)
                && let Some((card, caps)) = card_name(&file)
                && card.contains("Tiny 2 Lite")
                && caps & V4L2_CAP_META_CAPTURE == 0
            {
                return Ok(Self(file));
            }
        }
        Err(Error::NoCamera)
    }

    fn xu(&self, selector: u8, query: u8, data: &mut [u8]) -> Result<()> {
        let mut q = UvcXuControlQuery {
            unit: 2,
            selector,
            query,
            size: data.len() as u16,
            data: data.as_mut_ptr(),
        };
        unsafe { uvcioc_ctrl_query(self.0.as_raw_fd(), &mut q) }?;
        Ok(())
    }

    fn xu_len(&self, selector: u8) -> Result<usize> {
        let mut len = [0u8; 2];
        self.xu(selector, UVC_GET_LEN, &mut len)?;
        Ok(u16::from_le_bytes(len) as usize)
    }

    /// GET_CUR on unit 2, after checking the buffer length like the vendor tools do.
    pub fn xu_get(&self, selector: u8) -> Result<[u8; XU_SIZE]> {
        let len = self.xu_len(selector)?;
        if len > XU_SIZE {
            return Err(Error::Unsupported(format!(
                "selector {selector:#x} is {len} bytes"
            )));
        }
        let mut data = [0u8; XU_SIZE];
        self.xu(selector, UVC_GET_CUR, &mut data)?;
        Ok(data)
    }

    /// SET_CUR on unit 2 with a zero-padded 60-byte buffer.
    pub fn xu_set(&self, selector: u8, payload: &[u8]) -> Result<()> {
        if payload.len() > XU_SIZE {
            return Err(Error::Invalid("payload longer than 60 bytes".into()));
        }
        let len = self.xu_len(selector)?;
        if XU_SIZE > len {
            return Err(Error::Unsupported(format!(
                "selector {selector:#x} is {len} bytes"
            )));
        }
        let mut data = [0u8; XU_SIZE];
        data[..payload.len()].copy_from_slice(payload);
        self.xu(selector, UVC_SET_CUR, &mut data)
    }

    pub fn get_ctrl(&self, id: u32) -> Result<i32> {
        let mut c = V4l2Control { id, value: 0 };
        unsafe { vidioc_g_ctrl(self.0.as_raw_fd(), &mut c) }?;
        Ok(c.value)
    }

    pub fn set_ctrl(&self, id: u32, value: i32) -> Result<()> {
        let mut c = V4l2Control { id, value };
        unsafe { vidioc_s_ctrl(self.0.as_raw_fd(), &mut c) }?;
        Ok(())
    }

    /// Range of a control, or None when the camera does not have it (or it is disabled).
    pub fn ctrl_range(&self, id: u32) -> Result<Option<CtrlRange>> {
        let mut q = V4l2QueryCtrl {
            id,
            ..Default::default()
        };
        match unsafe { vidioc_queryctrl(self.0.as_raw_fd(), &mut q) } {
            Ok(_) if q.flags & V4L2_CTRL_FLAG_DISABLED == 0 => Ok(Some(CtrlRange {
                min: q.minimum,
                max: q.maximum,
                step: q.step,
                default: q.default_value,
            })),
            Ok(_) | Err(nix::errno::Errno::EINVAL) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
