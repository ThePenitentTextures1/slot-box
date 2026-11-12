use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use slot::frontend::Frontend;
use slot::manual_resize::{resize_page, MAX_HEIGHT};
use slot_power::SimPlatform;
use tempfile::tempdir;

fn png(width: u32, height: u32, depth: png::BitDepth) -> Vec<u8> {
    let bytes_per_pixel = if depth == png::BitDepth::Sixteen {
        8
    } else {
        4
    };
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![
                128;
                width as usize * height as usize * bytes_per_pixel
            ])
            .unwrap();
        writer.finish().unwrap();
    }
    out
}

fn dimensions(path: &Path) -> (u32, u32) {
    let bytes = fs::read(path).unwrap();
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let reader = decoder.read_info().unwrap();
    (reader.info().width, reader.info().height)
}

#[test]
fn downsizes_to_844_and_is_idempotent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("page.png");
    fs::write(&path, png(1920, 1440, png::BitDepth::Eight)).unwrap();
    assert!(resize_page(&path).unwrap());
    assert_eq!(dimensions(&path), (1125, MAX_HEIGHT));
    let resized = fs::read(&path).unwrap();
    assert!(!resize_page(&path).unwrap());
    assert_eq!(fs::read(path).unwrap(), resized);
}

#[test]
fn previously_resized_960_pixel_pages_are_reduced_to_the_new_limit() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("page.png");
    fs::write(&path, png(1280, 960, png::BitDepth::Eight)).unwrap();
    assert!(resize_page(&path).unwrap());
    assert_eq!(dimensions(&path), (1125, 844));
}

#[test]
fn pages_at_or_below_the_limit_are_unchanged() {
    let dir = tempdir().unwrap();
    for (width, height) in [(30, 843), (30, 844), (1200, 500)] {
        let path = dir.path().join(format!("{height}.png"));
        let original = png(width, height, png::BitDepth::Eight);
        fs::write(&path, &original).unwrap();
        assert!(!resize_page(&path).unwrap());
        assert_eq!(fs::read(path).unwrap(), original);
    }
}

#[test]
fn corrupt_files_are_not_overwritten() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("page.png");
    let original = b"not a PNG";
    fs::write(&path, original).unwrap();
    assert!(resize_page(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn sixteen_bit_pages_keep_their_color_depth() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("page.png");
    fs::write(&path, png(4, 1920, png::BitDepth::Sixteen)).unwrap();
    assert!(resize_page(&path).unwrap());
    let bytes = fs::read(&path).unwrap();
    let reader = png::Decoder::new(bytes.as_slice()).read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (1, 844));
    assert_eq!(reader.info().bit_depth, png::BitDepth::Sixteen);
    let (pixels, _, _) = slot_ui::decode_png16_bytes(&bytes).unwrap();
    assert!(pixels.iter().all(|&pixel| pixel == 0x8080));
}

#[test]
fn narrow_pages_keep_the_height_limit_and_nonzero_width() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("page.png");
    fs::write(&path, png(1, 2000, png::BitDepth::Eight)).unwrap();
    assert!(resize_page(&path).unwrap());
    assert_eq!(dimensions(&path), (1, 844));
}

#[test]
fn color_gamma_is_preserved() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("page.png");
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1920);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_source_gamma(png::ScaledFloat::new(0.45455));
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&vec![128; 2 * 1920 * 3]).unwrap();
        writer.finish().unwrap();
    }
    let before = png::Decoder::new(bytes.as_slice())
        .read_info()
        .unwrap()
        .info()
        .source_gamma;
    fs::write(&path, bytes).unwrap();
    assert!(resize_page(&path).unwrap());
    let after_bytes = fs::read(&path).unwrap();
    let reader = png::Decoder::new(after_bytes.as_slice())
        .read_info()
        .unwrap();
    assert_eq!(reader.info().source_gamma, before);
}

#[test]
fn frontend_boot_resizes_only_pages_for_discovered_games() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Games/GBA")).unwrap();
    fs::write(root.join("Games/GBA/Game.gba"), vec![0; 256]).unwrap();
    let manual = root.join("Manuals/GBA/Game");
    fs::create_dir_all(&manual).unwrap();
    let bad_page = manual.join("Game_page_1_image_1.png");
    let page = manual.join("Game_page_2_image_1.png");
    let unrelated = manual.join("Game_page_1_image_2.png");
    let original = png(12, 1440, png::BitDepth::Eight);
    fs::write(&bad_page, b"not a PNG").unwrap();
    fs::write(&page, &original).unwrap();
    fs::write(&unrelated, &original).unwrap();
    let frontend = Frontend::boot(Box::new(SimPlatform::at(root.to_path_buf())));
    let until = Instant::now() + Duration::from_secs(10);
    while dimensions(&page).1 != MAX_HEIGHT && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(dimensions(&page), (7, MAX_HEIGHT));
    assert_eq!(fs::read(bad_page).unwrap(), b"not a PNG");
    assert_eq!(fs::read(unrelated).unwrap(), original);
    drop(frontend);
}

#[test]
fn abandoned_manual_resize_temps_are_cleaned_up() {
    let dir = tempdir().unwrap();
    let manual = dir.path().join("Manuals/GBA/Game");
    fs::create_dir_all(&manual).unwrap();
    let old_temp = manual.join(".Game_page_1_image_1.png.0.1.tmp");
    let page = manual.join("Game_page_1_image_1.png");
    fs::write(&old_temp, b"partial").unwrap();
    fs::write(&page, b"original").unwrap();
    slot::root::sweep_temps(dir.path());
    assert!(!old_temp.exists());
    assert_eq!(fs::read(page).unwrap(), b"original");
}
