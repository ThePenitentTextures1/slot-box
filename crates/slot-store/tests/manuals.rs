use std::fs;
use std::path::Path;

use slot_store::{manual_pages, Cart, Platform};
use tempfile::tempdir;

fn cart(platform: Platform, stem: &str) -> Cart {
    Cart {
        platform,
        stem: stem.to_owned(),
        rom: "unused.gba".into(),
        label: None,
        title: String::new(),
        code: String::new(),
        shell: None,
    }
}

fn page(dir: &Path, name: &str) {
    fs::write(dir.join(name), b"not decoded during discovery").unwrap();
}

#[test]
fn finds_pages_in_numeric_order_with_case_insensitive_names() {
    let root = tempdir().unwrap();
    let dir = root.path().join("Manuals/GBA/Herbie_-_Fully_Loaded_USA");
    fs::create_dir_all(&dir).unwrap();
    for name in [
        "Herbie_-_Fully_Loaded_USA_page_10_image_1.png",
        "herbie_-_fully_loaded_usa_PAGE_2_IMAGE_1.PNG",
        "Herbie_-_Fully_Loaded_USA_page_001_image_1.png",
    ] {
        page(&dir, name);
    }
    let pages = manual_pages(
        root.path(),
        &cart(Platform::Gba, "herbie_-_fully_loaded_usa"),
    )
    .unwrap();
    assert_eq!(
        pages.iter().map(|p| p.number).collect::<Vec<_>>(),
        [1, 2, 10]
    );
    assert_eq!(
        pages[1].path,
        dir.join("herbie_-_fully_loaded_usa_PAGE_2_IMAGE_1.PNG")
    );
}

#[test]
fn missing_manuals_and_other_platforms_are_empty() {
    let root = tempdir().unwrap();
    assert!(manual_pages(root.path(), &cart(Platform::Gb, "Game"))
        .unwrap()
        .is_empty());
    let dir = root.path().join("Manuals/GBA/Game");
    fs::create_dir_all(&dir).unwrap();
    page(&dir, "Game_page_1_image_1.png");
    assert!(manual_pages(root.path(), &cart(Platform::Gb, "Game"))
        .unwrap()
        .is_empty());
    assert!(manual_pages(root.path(), &cart(Platform::Gba, "Other"))
        .unwrap()
        .is_empty());
    assert_eq!(
        manual_pages(root.path(), &cart(Platform::Gba, "Game"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn only_matching_page_images_are_discovered() {
    let root = tempdir().unwrap();
    let dir = root.path().join("Manuals/GBC/Game");
    fs::create_dir_all(&dir).unwrap();
    for name in [
        "Game_page_0_image_1.png",
        "Game_page_2_image_1.png",
        "Game_page_3_image_2.png",
        "Game_page_x_image_1.png",
        "Game_page__image_1.png",
        "Game_page_-1_image_1.png",
        "Game_page_4_image_1.jpg",
        "Game_page_5_image_1.png.bak",
        "Other_page_6_image_1.png",
        ".Game_page_7_image_1.png",
    ] {
        page(&dir, name);
    }
    fs::create_dir(dir.join("Game_page_8_image_1.png")).unwrap();
    let pages = manual_pages(root.path(), &cart(Platform::Gbc, "Game")).unwrap();
    assert_eq!(pages.iter().map(|p| p.number).collect::<Vec<_>>(), [0, 2]);
}

#[test]
fn duplicate_page_numbers_are_an_error() {
    let root = tempdir().unwrap();
    let dir = root.path().join("Manuals/GB/Game");
    fs::create_dir_all(&dir).unwrap();
    page(&dir, "Game_page_1_image_1.png");
    page(&dir, "Game_page_01_image_1.png");
    let err = manual_pages(root.path(), &cart(Platform::Gb, "Game")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn a_page_number_that_does_not_fit_is_an_error() {
    let root = tempdir().unwrap();
    let dir = root.path().join("Manuals/GBAV/Game");
    fs::create_dir_all(&dir).unwrap();
    page(&dir, "Game_page_18446744073709551616_image_1.png");
    let err = manual_pages(root.path(), &cart(Platform::Gbav, "Game")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn case_colliding_game_directories_are_an_error() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("Manuals/GBA/Game")).unwrap();
    fs::create_dir_all(root.path().join("Manuals/GBA/GAME")).unwrap();
    let err = manual_pages(root.path(), &cart(Platform::Gba, "game")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn filesystem_errors_are_not_reported_as_missing_manuals() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("Manuals")).unwrap();
    fs::write(root.path().join("Manuals/GBA"), b"not a directory").unwrap();
    assert!(manual_pages(root.path(), &cart(Platform::Gba, "Game")).is_err());
}
