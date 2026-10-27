// SPDX-License-Identifier: GPL-3.0-or-later

//! State file for values the camera does not report: the last gimbal pose sent and the
//! exposure mode. The CLI updates it whenever it sets them, so separate calls stay
//! consistent. Location: `$XDG_STATE_HOME/obsbot4linux/state.json` (default `~/.local/state`).

use crate::commands::ExposureMode;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;
use std::{env, fs, process};

/// The daemon keeps the state in memory; `save` still writes the file, and `--direct` calls
/// write it too, so the copy is read again when the file changed.
static CACHING: AtomicBool = AtomicBool::new(false);
static CACHE: Cache = Cache(Mutex::new(None));

/// Turns on the in-memory copy; call once, in the daemon.
pub fn enable_cache() {
    CACHING.store(true, Ordering::Relaxed);
}

/// What tells a changed file apart: every save renames a new file (new inode) into place.
type Stamp = Option<(u64, SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let m = fs::metadata(path).ok()?;
    Some((m.ino(), m.modified().ok()?, m.len()))
}

/// A missing or unreadable file is an empty state.
fn read(path: &Path) -> State {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Writes atomically: a temporary file of its own (two `--direct` calls at once must not rename
/// each other's half-written file), synced so a power loss cannot leave an empty file, renamed.
fn write(state: &State, path: &Path) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path
        .parent()
        .ok_or_else(|| Error::Invalid("bad state path".into()))?;
    fs::create_dir_all(dir)?;
    let data = serde_json::to_vec_pretty(state).map_err(|e| Error::Invalid(e.to_string()))?;
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}.{n}.tmp", process::id()));
    let written = File::create(&tmp)
        .and_then(|mut f| f.write_all(&data).and_then(|_| f.sync_all()))
        .and_then(|_| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(written?)
}

/// The state with the stamp of the file it was read from or written to.
struct Cache(Mutex<Option<(State, Stamp)>>);

impl Cache {
    fn load(&self, path: &Path) -> State {
        let mut entry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = stamp(path);
        if let Some((state, at)) = entry.as_ref()
            && *at == now
        {
            return state.clone();
        }
        let state = read(path);
        *entry = Some((state.clone(), now));
        state
    }

    /// The copy changes only once the file is written: a failed write leaves both as they were.
    fn save(&self, state: &State, path: &Path) -> Result<()> {
        let mut entry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        write(state, path)?;
        *entry = Some((state.clone(), stamp(path)));
        Ok(())
    }
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
}

/// The start pose (initial state, applied by the camera at power-on), read from query `0x3884`.
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
    base.join("obsbot4linux/state.json")
}

impl State {
    /// Loads the state (memory copy if enabled and the file is unchanged, else the file); a
    /// missing or unreadable file is an empty state.
    pub fn load() -> Self {
        if CACHING.load(Ordering::Relaxed) {
            CACHE.load(&path())
        } else {
            read(&path())
        }
    }

    /// Writes the file atomically (temporary file, then rename).
    pub fn save(&self) -> Result<()> {
        if CACHING.load(Ordering::Relaxed) {
            CACHE.save(self, &path())
        } else {
            write(self, &path())
        }
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

    fn file_in(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("o4l-state-{tag}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("state.json")
    }

    fn posed(pan: f32) -> State {
        State::default().with_gimbal(Some(pan), Some(0.0))
    }

    #[test]
    fn the_cache_sees_a_direct_write() {
        let path = file_in("direct");
        let cache = Cache(Mutex::new(None));
        cache.save(&posed(10.0), &path).unwrap();
        assert_eq!(cache.load(&path), posed(10.0));
        // `--direct gimbal --pan 20` in another process; same length, so mtime or inode tell
        write(&posed(20.0), &path).unwrap();
        assert_eq!(cache.load(&path), posed(20.0));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_failed_write_leaves_the_cache_and_no_temporary_file() {
        let path = file_in("failed");
        let cache = Cache(Mutex::new(None));
        cache.save(&posed(10.0), &path).unwrap();
        // the rename fails: a non-empty directory is in the way
        fs::remove_file(&path).unwrap();
        fs::create_dir_all(path.join("x")).unwrap();
        assert!(cache.save(&posed(20.0), &path).is_err());
        let entry = cache.0.lock().unwrap();
        assert_eq!(entry.as_ref().map(|(s, _)| s), Some(&posed(10.0)));
        let dir = fs::read_dir(path.parent().unwrap()).unwrap();
        let names: Vec<_> = dir.flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, ["state.json"]);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
