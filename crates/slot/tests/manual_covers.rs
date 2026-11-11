use std::fs;
use std::path::Path;

use slot::manual_covers::load_cover;
use slot_store::{Cart, Platform};
use tempfile::tempdir;

fn cart(stem: &str) -> Cart {
    Cart {
        platform: Platform::Gba,
        stem: stem.into(),
        rom: "game.gba".into(),
        label: None,
        title: String::new(),
        code: String::new(),
        shell: None,
    }
}

fn png(path: &Path, width: u32, height: u32, colour: [u8; 3]) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&colour.repeat((width * height) as usize))
        .unwrap();
}

#[test]
fn discovers_and_decodes_the_first_page_in_numeric_order() {
    let root = tempdir().unwrap();
    let dir = root.path().join("Manuals/GBA/Game");
    fs::create_dir_all(&dir).unwrap();
    png(&dir.join("Game_page_10_image_1.png"), 2, 2, [0, 0, 255]);
    png(&dir.join("Game_page_2_image_1.png"), 2, 2, [255, 0, 0]);
    let cover = load_cover(root.path(), &cart("Game")).unwrap().unwrap();
    assert_eq!((cover.w, cover.h), (2, 2));
    assert_eq!(&cover.rgba[..4], &[255, 0, 0, 255]);
}

#[test]
fn missing_cover_is_absent_and_invalid_cover_is_an_error() {
    let root = tempdir().unwrap();
    assert!(load_cover(root.path(), &cart("Game")).unwrap().is_none());
    let dir = root.path().join("Manuals/GBA/Game");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Game_page_1_image_1.png"), b"broken").unwrap();
    assert!(load_cover(root.path(), &cart("Game")).is_err());
}

#[test]
fn cover_is_bounded_even_if_background_resizing_is_not_yet_done() {
    let root = tempdir().unwrap();
    let dir = root.path().join("Manuals/GBA/Game");
    fs::create_dir_all(&dir).unwrap();
    png(&dir.join("Game_page_1_image_1.png"), 12, 1440, [10, 20, 30]);
    let cover = load_cover(root.path(), &cart("Game")).unwrap().unwrap();
    assert_eq!((cover.w, cover.h), (8, 960));
    assert_eq!(&cover.rgba[..4], &[10, 20, 30, 255]);
}
