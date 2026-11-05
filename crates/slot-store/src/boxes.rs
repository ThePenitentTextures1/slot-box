use std::path::{Path, PathBuf};

use crate::{Cart, Platform};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoxArtSource {
    Separate,
    Texture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxArt {
    pub platform: Platform,
    pub stem: String,
    pub source: BoxArtSource,
    pub front: Option<PathBuf>,
    pub side: Option<PathBuf>,
    pub back: Option<PathBuf>,
    pub texture: Option<PathBuf>,
}

impl BoxArt {
    pub fn is_texture(&self) -> bool {
        matches!(self.source, BoxArtSource::Texture)
    }

    pub fn has_parts(&self) -> bool {
        self.front.is_some() && self.side.is_some() && self.back.is_some()
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

pub fn box_art(root: &Path, cart: &Cart) -> Option<BoxArt> {
    let dir = box_dir(root, cart.platform);
    let separate = (
        box_art_path(root, cart, "front"),
        box_art_path(root, cart, "side"),
        box_art_path(root, cart, "back"),
    );
    if separate.0.is_file() && separate.1.is_file() && separate.2.is_file() {
        return Some(BoxArt {
            platform: cart.platform,
            stem: cart.stem.clone(),
            source: BoxArtSource::Separate,
            front: Some(separate.0),
            side: Some(separate.1),
            back: Some(separate.2),
            texture: None,
        });
    }

    let texture = box_art_path(root, cart, "texture");
    if texture.is_file() {
        return Some(BoxArt {
            platform: cart.platform,
            stem: cart.stem.clone(),
            source: BoxArtSource::Texture,
            front: None,
            side: None,
            back: None,
            texture: Some(texture),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cart(platform: Platform, stem: &str) -> Cart {
        Cart {
            platform,
            stem: stem.to_string(),
            rom: PathBuf::from(format!("/tmp/{stem}.gba")),
            label: None,
            title: String::new(),
            code: String::new(),
            shell: None,
        }
    }

    #[test]
    fn separate_box_files_are_preferred() {
        let d = tempfile::tempdir().unwrap();
        let c = cart(Platform::Gba, "mario");
        let dir = box_dir(d.path(), c.platform);
        fs::create_dir_all(&dir).unwrap();
        for name in ["front", "side", "back", "texture"] {
            let path = dir.join(format!("mario-{name}.png"));
            fs::write(path, b"png").unwrap();
        }

        let art = box_art(d.path(), &c).unwrap();
        assert_eq!(art.source, BoxArtSource::Separate);
        assert_eq!(art.front.as_deref(), Some(dir.join("mario-front.png").as_path()));
        assert_eq!(art.side.as_deref(), Some(dir.join("mario-side.png").as_path()));
        assert_eq!(art.back.as_deref(), Some(dir.join("mario-back.png").as_path()));
        assert_eq!(art.texture, None);
    }

    #[test]
    fn a_texture_is_used_when_the_separate_files_are_missing() {
        let d = tempfile::tempdir().unwrap();
        let c = cart(Platform::Gba, "zelda");
        let dir = box_dir(d.path(), c.platform);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("zelda-texture.png"), b"png").unwrap();

        let art = box_art(d.path(), &c).unwrap();
        assert_eq!(art.source, BoxArtSource::Texture);
        assert_eq!(art.texture, Some(dir.join("zelda-texture.png")));
        assert_eq!(art.front, None);
        assert_eq!(art.side, None);
        assert_eq!(art.back, None);
    }

    #[test]
    fn missing_box_files_fall_back_to_the_default_cartridge_view() {
        let d = tempfile::tempdir().unwrap();
        let c = cart(Platform::Gba, "metroid");
        assert!(box_art(d.path(), &c).is_none());
    }
}
