use std::path::{Path, PathBuf};

use crate::atomic::atomic_write;

/// The selected visual treatment for the game carousel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CarouselView {
    /// Existing default: display games as cartridges.
    #[default]
    Cartridges,
    /// Display games using their optional box art, falling back to cartridges.
    Boxes,
}

impl CarouselView {
    pub const fn key(self) -> &'static str {
        match self {
            CarouselView::Cartridges => "cartridges",
            CarouselView::Boxes => "boxes",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            CarouselView::Cartridges => "Cartridges",
            CarouselView::Boxes => "Boxes",
        }
    }

    pub const fn toggle(self) -> Self {
        match self {
            CarouselView::Cartridges => CarouselView::Boxes,
            CarouselView::Boxes => CarouselView::Cartridges,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "cartridges" => Some(CarouselView::Cartridges),
            "boxes" => Some(CarouselView::Boxes),
            _ => None,
        }
    }
}

const FILE: &str = "Config/carousel_view.txt";

pub fn read(root: &Path) -> CarouselView {
    std::fs::read_to_string(root.join(FILE))
        .ok()
        .and_then(|value| CarouselView::parse(&value))
        .unwrap_or_default()
}

pub fn write(root: &Path, view: CarouselView) -> std::io::Result<()> {
    atomic_write(&path(root), format!("{}\n", view.key()).as_bytes())
}

pub fn path(root: &Path) -> PathBuf {
    root.join(FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_existing_cartridge_view() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read(d.path()), CarouselView::Cartridges);
    }

    #[test]
    fn setting_round_trips() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), CarouselView::Boxes).unwrap();
        assert_eq!(read(d.path()), CarouselView::Boxes);
        write(d.path(), CarouselView::Cartridges).unwrap();
        assert_eq!(read(d.path()), CarouselView::Cartridges);
    }

    #[test]
    fn an_unknown_value_defaults_to_cartridges() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Config")).unwrap();
        std::fs::write(path(d.path()), "future-view\n").unwrap();
        assert_eq!(read(d.path()), CarouselView::Cartridges);
    }
}
