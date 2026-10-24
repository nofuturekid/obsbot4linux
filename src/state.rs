// SPDX-License-Identifier: GPL-3.0-or-later

//! State file for values the camera does not report: the last gimbal pose sent and the
//! exposure mode. The CLI updates it whenever it sets them, so separate calls stay
//! consistent. Location: `$XDG_STATE_HOME/t2l/state.json` (default `~/.local/state`).

use crate::commands::ExposureMode;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{env, fs};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Gimbal {
    /// Degrees, positive turns the camera to its left.
    pub pan: Option<f32>,
    /// Degrees, positive tilts up.
    pub tilt: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Last pose sent; None when unknown (never set, or tracking may have moved it since).
    pub gimbal: Option<Gimbal>,
    pub exposure_mode: Option<ExposureMode>,
}

pub fn path() -> PathBuf {
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("t2l/state.json")
}

impl State {
    /// Loads the file; a missing or unreadable file is an empty state.
    pub fn load() -> Self {
        fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Writes the file atomically (temporary file, then rename).
    pub fn save(&self) -> Result<()> {
        let path = path();
        let dir = path.parent().ok_or_else(|| Error::Invalid("bad state path".into()))?;
        fs::create_dir_all(dir)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    /// Merges a gimbal move: an axis that was not moved keeps its remembered value.
    pub fn with_gimbal(mut self, pan: Option<f32>, tilt: Option<f32>) -> Self {
        let old = self.gimbal.unwrap_or(Gimbal { pan: None, tilt: None });
        self.gimbal = Some(Gimbal { pan: pan.or(old.pan), tilt: tilt.or(old.tilt) });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_of_one_axis_keeps_the_other() {
        let s = State::default().with_gimbal(Some(10.0), Some(5.0)).with_gimbal(None, Some(-3.0));
        assert_eq!(s.gimbal, Some(Gimbal { pan: Some(10.0), tilt: Some(-3.0) }));
    }

    #[test]
    fn json_round_trip() {
        let s = State { gimbal: None, exposure_mode: Some(ExposureMode::Face) };
        let back: State = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
