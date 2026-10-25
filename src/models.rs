// SPDX-License-Identifier: GPL-3.0-or-later

//! The cameras obsbot4linux supports and how to recognise them. Matching is pure string
//! work so it can be tested without a camera; `device::detect` feeds it from `/dev`.

/// A supported camera model.
#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    /// Human name, reported as `model`.
    pub name: &'static str,
    /// USB vendor:product id.
    pub usb_id: &'static str,
    /// Part of the `/dev/v4l/by-id` link name (udev's product string).
    pub by_id: &'static str,
    /// Part of the V4L2 card name, for the fallback when there is no by-id link.
    pub card: &'static str,
}

pub const MODELS: &[Model] = &[Model {
    name: "Tiny 2 Lite",
    usb_id: "3564:fef9",
    by_id: "OBSBOT_Tiny_2_Lite",
    card: "Tiny 2 Lite",
}];

/// The model a `/dev/v4l/by-id` link name belongs to; only the streaming node
/// (`-video-index0`), not the metadata node.
pub fn from_by_id(link: &str) -> Option<&'static Model> {
    if !link.ends_with("-video-index0") {
        return None;
    }
    MODELS.iter().find(|m| link.contains(m.by_id))
}

/// The model a V4L2 card name belongs to.
pub fn from_card(card: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| card.contains(m.card))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_id_takes_the_video_node_not_the_metadata_node() {
        let base = "usb-Remo_Tech_Co.__Ltd._OBSBOT_Tiny_2_Lite";
        let m = from_by_id(&format!("{base}-video-index0")).unwrap();
        assert_eq!(m.name, "Tiny 2 Lite");
        assert_eq!(m.usb_id, "3564:fef9");
        assert!(from_by_id(&format!("{base}-video-index1")).is_none());
    }

    #[test]
    fn other_cameras_are_not_matched() {
        assert!(from_by_id("usb-Logitech_BRIO-video-index0").is_none());
        assert!(from_card("Integrated Camera").is_none());
        assert_eq!(from_card("OBSBOT Tiny 2 Lite").unwrap().name, "Tiny 2 Lite");
    }
}
