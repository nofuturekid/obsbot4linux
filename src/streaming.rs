// SPDX-License-Identifier: GPL-3.0-or-later

//! Which applications stream from the camera.
//!
//! The USB alternate setting does not tell (the camera streams in bulk mode). A process that
//! streams from a V4L2 node has it open and mmaps its buffers, so the node shows up in
//! `/proc/<pid>/maps`; a process that only holds the node open (a control panel) does not
//! map it. Capture through DMABUF or userptr buffers maps nothing and is not seen, and
//! neither are processes of other users (their `/proc` entries are unreadable).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// `comm` names (sorted, deduplicated) of the processes under `proc_root` that have `node`
/// open and mapped. `node` must be the resolved `/dev/videoN` path; `own_pid` is skipped.
/// Unreadable processes are ignored.
pub fn streaming_apps(proc_root: &Path, node: &Path, own_pid: u32) -> Vec<String> {
    let mut apps = BTreeSet::new();
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let dir = entry.path();
        if has_open(&dir, node)
            && maps_node(&dir, node)
            && let Ok(comm) = fs::read_to_string(dir.join("comm"))
        {
            apps.insert(comm.trim_end().to_string());
        }
    }
    apps.into_iter().collect()
}

/// Applications streaming from `device` (a `/dev/videoN` path or a symlink to it), looked up
/// in the real `/proc`, without this process.
pub fn apps_using(device: &Path) -> Vec<String> {
    let node = fs::canonicalize(device).unwrap_or_else(|_| device.to_path_buf());
    streaming_apps(Path::new("/proc"), &node, std::process::id())
}

fn has_open(proc_dir: &Path, node: &Path) -> bool {
    let Ok(fds) = fs::read_dir(proc_dir.join("fd")) else {
        return false;
    };
    fds.flatten()
        .any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target == node))
}

fn maps_node(proc_dir: &Path, node: &Path) -> bool {
    let Some(node) = node.to_str() else {
        return false;
    };
    let Ok(maps) = fs::read_to_string(proc_dir.join("maps")) else {
        return false;
    };
    // the path is the last column; "(deleted)" can follow it
    maps.lines().any(|l| {
        l.split_once('/')
            .is_some_and(|(_, rest)| format!("/{rest}").trim_end_matches(" (deleted)") == node)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    const NODE: &str = "/dev/video0";

    struct Fake(PathBuf);

    impl Fake {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("o4l-proc-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Fake(root)
        }

        /// A process with the given fd targets and maps text.
        fn process(&self, pid: u32, comm: &str, fds: &[&str], maps: &str) {
            let dir = self.0.join(pid.to_string());
            fs::create_dir_all(dir.join("fd")).unwrap();
            fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
            fs::write(dir.join("maps"), maps).unwrap();
            for (i, target) in fds.iter().enumerate() {
                symlink(target, dir.join("fd").join(i.to_string())).unwrap();
            }
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mapped() -> String {
        format!("7f00-7f10 rw-s 00000000 00:06 1234   {NODE}\n")
    }

    fn apps(fake: &Fake) -> Vec<String> {
        streaming_apps(&fake.0, Path::new(NODE), 1)
    }

    #[test]
    fn idle_when_nobody_has_the_node() {
        let f = Fake::new("idle");
        f.process(10, "bash", &["/dev/pts/1", "/dev/video2"], "");
        assert!(apps(&f).is_empty());
    }

    #[test]
    fn open_without_mapping_is_not_streaming() {
        // a control panel holds the node to send commands, it does not capture
        let f = Fake::new("open");
        f.process(
            10,
            "qs",
            &[NODE],
            "7f00-7f10 r-xp 0 00:01 5 /usr/lib/libc.so\n",
        );
        assert!(apps(&f).is_empty());
    }

    #[test]
    fn open_and_mapped_is_streaming() {
        let f = Fake::new("mapped");
        f.process(10, "ffmpeg", &[NODE], &mapped());
        assert_eq!(apps(&f), ["ffmpeg"]);
    }

    #[test]
    fn mapped_but_not_open_is_not_streaming() {
        // the fd scan comes first; a stale or foreign mapping alone does not count
        let f = Fake::new("nofd");
        f.process(10, "ffmpeg", &["/dev/null"], &mapped());
        assert!(apps(&f).is_empty());
    }

    #[test]
    fn two_apps_are_sorted_and_deduplicated() {
        let f = Fake::new("two");
        f.process(30, "obs", &[NODE], &mapped());
        f.process(20, "chrome", &[NODE], &mapped());
        f.process(25, "chrome", &[NODE], &mapped());
        assert_eq!(apps(&f), ["chrome", "obs"]);
    }

    #[test]
    fn unreadable_and_odd_entries_are_ignored() {
        let f = Fake::new("odd");
        fs::create_dir_all(f.0.join("77")).unwrap(); // no fd directory, like another user's
        fs::create_dir_all(f.0.join("self")).unwrap();
        fs::write(f.0.join("uptime"), "1 1").unwrap();
        // fds but no readable maps
        let dir = f.0.join("88");
        fs::create_dir_all(dir.join("fd")).unwrap();
        symlink(NODE, dir.join("fd/0")).unwrap();
        f.process(10, "ffmpeg", &[NODE], &mapped());
        assert_eq!(apps(&f), ["ffmpeg"]);
    }

    #[test]
    fn the_own_process_is_excluded() {
        let f = Fake::new("own");
        f.process(1, "obsbot4linux", &[NODE], &mapped());
        f.process(10, "ffmpeg", &[NODE], &mapped());
        assert_eq!(apps(&f), ["ffmpeg"]);
    }

    #[test]
    fn a_similar_node_name_does_not_match() {
        let f = Fake::new("similar");
        f.process(
            10,
            "ffmpeg",
            &["/dev/video01"],
            "7f00-7f10 rw-s 0 00:06 1 /dev/video01\n",
        );
        assert!(apps(&f).is_empty());
    }

    #[test]
    fn missing_proc_root_is_idle() {
        assert!(streaming_apps(Path::new("/nonexistent-proc"), Path::new(NODE), 1).is_empty());
    }
}
