// SPDX-License-Identifier: GPL-3.0-or-later

//! State file for values the camera does not report: the last gimbal pose sent and the
//! exposure mode. The CLI updates it whenever it sets them, so separate calls stay
//! consistent. Location: `$XDG_STATE_HOME/t2l/state.json` (default `~/.local/state`).

use crate::commands::ExposureMode;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{env, fs};

/// The daemon keeps the state in memory (it is the only writer); `save` still writes the file.
static CACHING: AtomicBool = AtomicBool::new(false);
static CACHE: Mutex<Option<State>> = Mutex::new(None);

/// Turns on the in-memory copy; call once in a process that is the only writer.
pub fn enable_cache() {
    CACHING.store(true, Ordering::Relaxed);
}

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
    /// Zone tracking "follow new targets"; the camera has no readback for it.
    pub zone_follow: Option<bool>,
    /// Start pose last sent as initial state (applied by the camera at power-on); no readback.
    pub startup: Option<Startup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Startup {
    /// Preset slot the start pose was taken from, if any.
    pub preset: Option<u8>,
    pub pan: f32,
    pub tilt: f32,
    pub zoom: f32,
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
    /// Loads the state (memory copy if enabled, else the file); a missing or unreadable
    /// file is an empty state.
    pub fn load() -> Self {
        let caching = CACHING.load(Ordering::Relaxed);
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if caching && let Some(state) = cache.as_ref() {
            return state.clone();
        }
        let state: State = fs::read(path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if caching {
            *cache = Some(state.clone());
        }
        state
    }

    /// Writes the file atomically (temporary file, then rename).
    pub fn save(&self) -> Result<()> {
        if CACHING.load(Ordering::Relaxed) {
            *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.clone());
        }
        let path = path();
        let dir = path
            .parent()
            .ok_or_else(|| Error::Invalid("bad state path".into()))?;
        fs::create_dir_all(dir)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?,
        )?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    /// Merges a gimbal move: an axis that was not moved keeps its remembered value.
    pub fn with_gimbal(mut self, pan: Option<f32>, tilt: Option<f32>) -> Self {
        let old = self.gimbal.unwrap_or(Gimbal {
            pan: None,
            tilt: None,
        });
        self.gimbal = Some(Gimbal {
            pan: pan.or(old.pan),
            tilt: tilt.or(old.tilt),
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_of_one_axis_keeps_the_other() {
        let s = State::default()
            .with_gimbal(Some(10.0), Some(5.0))
            .with_gimbal(None, Some(-3.0));
        assert_eq!(
            s.gimbal,
            Some(Gimbal {
                pan: Some(10.0),
                tilt: Some(-3.0)
            })
        );
    }

    #[test]
    fn json_round_trip() {
        let s = State {
            exposure_mode: Some(ExposureMode::Face),
            ..State::default()
        };
        let back: State = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
