// SPDX-License-Identifier: GPL-3.0-or-later

//! Error type shared by all modules.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no OBSBOT Tiny 2 Lite found")]
    NoCamera,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ioctl failed: {0}")]
    Ioctl(#[from] nix::errno::Errno),
    #[error("invalid value: {0}")]
    Invalid(String),
    #[error("camera did not answer query {0:#06x}")]
    NoAnswer(u16),
    #[error("camera rejected query {0:#06x} (error code {1:#06x})")]
    Rejected(u16, u16),
    #[error("{0}")]
    Unsupported(String),
    #[error("the camera is busy: another obsbot4linux process held it for 5 s")]
    Busy,
}

pub type Result<T> = std::result::Result<T, Error>;
