//! CapCut timeline integration boundary.
//! This module does not import, simulate mouse actions, modify CapCut project files,
//! or claim compatibility with undocumented CapCut internals.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedImage {
    pub sequence: u32,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteResult {
    Inserted,
    NotReady,
    Unsupported,
    Failed(String),
}

pub trait TimelineAdapter {
    fn is_ready(&self) -> bool;
    fn insert_at_playhead(&self, image: &Path) -> PasteResult;
}

/// Safe placeholder until a version-specific, tested CapCut adapter exists.
pub struct UnsupportedCapCutAdapter;
impl TimelineAdapter for UnsupportedCapCutAdapter {
    fn is_ready(&self) -> bool { false }
    fn insert_at_playhead(&self, _image: &Path) -> PasteResult {
        PasteResult::Unsupported
    }
}

pub struct TimelineBridge<A: TimelineAdapter> {
    latest: Option<SavedImage>,
    adapter: A,
}

impl<A: TimelineAdapter> TimelineBridge<A> {
    pub fn new(adapter: A) -> Self {
        Self { latest: None, adapter }
    }

    /// Called on the UI thread only after the worker has saved a real PNG.
    pub fn on_image_saved(&mut self, sequence: u32, path: PathBuf) {
        let newer = self.latest.as_ref().is_none_or(|last| {
            sequence.wrapping_sub(last.sequence) < 0x8000_0000
        });
        if newer {
            self.latest = Some(SavedImage { sequence, path });
        }
    }

    pub fn latest(&self) -> Option<&SavedImage> {
        self.latest.as_ref()
    }

    /// No disk scan, clipboard write, keyboard injection or mouse movement.
    pub fn on_paste(&self) -> PasteResult {
        if !self.adapter.is_ready() {
            return PasteResult::Unsupported;
        }
        match self.latest.as_ref() {
            Some(image) if image.path.is_file() =>
                self.adapter.insert_at_playhead(&image.path),
            _ => PasteResult::NotReady,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_adapter_never_touches_capcut() {
        let mut bridge = TimelineBridge::new(UnsupportedCapCutAdapter);
        bridge.on_image_saved(7, PathBuf::from("C:\\Images\\a.png"));
        assert_eq!(bridge.on_paste(), PasteResult::Unsupported);
    }
    #[test]
    fn newer_sequence_wins_even_across_wraparound() {
        let mut bridge = TimelineBridge::new(UnsupportedCapCutAdapter);
        bridge.on_image_saved(u32::MAX, PathBuf::from("first.png"));
        bridge.on_image_saved(0, PathBuf::from("second.png"));
        bridge.on_image_saved(u32::MAX - 2, PathBuf::from("old.png"));
        assert_eq!(bridge.latest().unwrap().path, PathBuf::from("second.png"));
    }
}
