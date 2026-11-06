use std::path::{Path, PathBuf};

use crate::{Cart, Platform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxArtSource {
    Separate,
    Texture,
}

/// The panel of a texture image used for one box face.
///
/// Texture panels are laid out back, side, front from left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexturePanel {
    Back,
    Side,
    Front,
}

/// A path to an image and, for texture sources, the panel to crop from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxArtImage {
    pub path: PathBuf,
    pub texture_panel: Option<TexturePanel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxArt {
    pub platform: Platform,
    pub stem: String,
    pub source: BoxArtSource,
    pub front: BoxArtImage,
    pub side: BoxArtImage,
    pub back: BoxArtImage,
}

impl BoxArt {
    pub fn is_texture(&self) -> bool {
        self.source == BoxArtSource::Texture
    }
}

pub fn box_root(root: &Path) -> PathBuf {
    root.join("Boxes")
}

pub fn box_dir(root: &Path, platform: Platform) -> PathBuf {
    box_root(root).join(platform.dir_name())
}

pub fn box_art_path(root: &Path, cart: &Cart, kind: &str) -> PathBuf {
    box_dir(root, cart.platform).join(format!("{}-{kind}.png", cart.stem))
}

/// Finds an asset by filename without depending on filesystem case sensitivity.
fn find_art_file(dir: &Path, filename: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries.flatten().find_map(|entry| {
        let name = entry.file_name();
        if name.to_str()?.eq_ignore_ascii_case(filename) && entry.path().is_file() {
            Some(entry.path())
        } else {
            None
        }
    })
}

fn separate_image(path: PathBuf) -> BoxArtImage {
    BoxArtImage {
        path,
        texture_panel: None,
    }
}

fn texture_image(path: PathBuf, panel: TexturePanel) -> BoxArtImage {
    BoxArtImage {
        path,
        texture_panel: Some(panel),
    }
}

/// Resolves a game's box art from Boxes/<platform>/<rom-stem>-*.png.
///
/// A complete set of separate front/side/back images takes precedence. If
/// that set is incomplete, a single texture image is used when present. The
/// returned texture panels are ordered back, side, front from left to right.
/// Missing art returns None, allowing the caller to show the loose cartridge.
pub fn box_art(root: &Path, cart: &Cart) -> Option<BoxArt> {
    let dir = box_dir(root, cart.platform);
    let front = find_art_file(&dir, &format!("{}-front.png", cart.stem));
    let side = find_art_file(&dir, &format!("{}-side.png", cart.stem));
    let back = find_art_file(&dir, &format!("{}-back.png", cart.stem));

    if let (Some(front), Some(side), Some(back)) = (front, side, back) {
        return Some(BoxArt {
            platform: cart.platform,
            stem: cart.stem.clone(),
            source: BoxArtSource::Separate,
            front: separate_image(front),
            side: separate_image(side),
            back: separate_image(back),
        });
    }

    let texture = find_art_file(&dir, &format!("{}-texture.png", cart.stem))?;
    Some(BoxArt {
        platform: cart.platform,
        stem: cart.stem.clone(),
        source: BoxArtSource::Texture,
        front: texture_image(texture.clone(), TexturePanel::Front),
        side: texture_image(texture.clone(), TexturePanel::Side),
        back: texture_image(texture, TexturePanel::Back),
    })
}
