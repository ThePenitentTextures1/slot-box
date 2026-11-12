//! Resize discovered manual pages in the background without blocking the shelf.
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use slot_store::Cart;

pub const MAX_HEIGHT: u32 = 844;
pub(crate) const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub struct Worker {
    cancel: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(root: PathBuf, carts: Vec<Cart>) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        if let Err(e) = std::thread::Builder::new()
            .name("slot-manual-resize".into())
            .spawn(move || {
                for cart in carts {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let pages = match slot_store::manual_pages(&root, &cart) {
                        Ok(pages) => pages,
                        Err(e) => {
                            eprintln!("slot: manual {}: {e}", cart.stem);
                            continue;
                        }
                    };
                    for page in pages {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Err(e) = resize_page(&page.path) {
                            eprintln!("slot: manual resize {}: {e}", page.path.display());
                        }
                    }
                }
            })
        {
            eprintln!("slot: manual resize: worker failed to start: {e}");
        }
        Self { cancel }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Atomically replaces an oversized PNG, returning whether it was resized.
pub fn resize_page(path: &Path) -> io::Result<bool> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "manual page is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "manual page exceeds the file size limit",
        ));
    }
    let mut decoder = png::Decoder::new(bytes.as_slice());
    decoder.set_limits(png::Limits { bytes: 64 << 20 });
    let reader = decoder.read_info().map_err(io::Error::other)?;
    let info = reader.info();
    if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "manual page exceeds the pixel limit",
        ));
    }
    if info.height <= MAX_HEIGHT && info.width <= MAX_HEIGHT {
        return Ok(false);
    }
    if info.animation_control.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "animated manual PNG cannot be resized safely",
        ));
    }
    let bit_depth = info.bit_depth;
    let source_color = info.color_type;
    let icc_profile = info.icc_profile.clone().map(|p| p.into_owned());
    let source_gamma = info.source_gamma;
    let source_chromaticities = info.source_chromaticities;
    let srgb = info.srgb;
    drop(reader);
    let (rgba, width, height) = slot_ui::decode_png_bytes(&bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "could not decode manual PNG"))?;
    if height <= MAX_HEIGHT {
        return Ok(false);
    }
    if icc_profile.is_some()
        && matches!(
            source_color,
            png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "manual PNG has a grayscale ICC profile that cannot be preserved",
        ));
    }
    let max_width = ((u64::from(width) * u64::from(MAX_HEIGHT)).div_ceil(u64::from(height))) as u32;
    let (resized, width, height) = if bit_depth == png::BitDepth::Sixteen {
        drop(rgba);
        let (rgba, width, height) = slot_ui::decode_png16_bytes(&bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "could not decode 16-bit manual PNG",
            )
        })?;
        let (pixels, width, height) = resize_16(&rgba, width, height, max_width);
        let bytes = pixels.into_iter().flat_map(u16::to_be_bytes).collect();
        (bytes, width, height)
    } else {
        slot_ui::thumbnail(&rgba, width, height, max_width, MAX_HEIGHT)
    };
    let mut output = Vec::new();
    {
        let mut info = png::Info::with_size(width, height);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = if bit_depth == png::BitDepth::Sixteen {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        };
        info.icc_profile = icc_profile.map(std::borrow::Cow::Owned);
        info.source_gamma = source_gamma;
        info.source_chromaticities = source_chromaticities;
        info.srgb = srgb;
        let encoder = png::Encoder::with_info(&mut output, info).map_err(io::Error::other)?;
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer
            .write_image_data(&resized)
            .map_err(io::Error::other)?;
        writer.finish().map_err(io::Error::other)?;
    }

    fn resize_16(src: &[u16], width: u32, height: u32, max_width: u32) -> (Vec<u16>, u32, u32) {
        let target_width = ((u64::from(width) * u64::from(MAX_HEIGHT)) / u64::from(height))
            .max(1)
            .min(u64::from(max_width)) as u32;
        let mut out = Vec::with_capacity(target_width as usize * MAX_HEIGHT as usize * 4);
        for y in 0..MAX_HEIGHT {
            let y0 = u64::from(y) * u64::from(height) / u64::from(MAX_HEIGHT);
            let y1 = u64::from(y + 1) * u64::from(height) / u64::from(MAX_HEIGHT);
            for x in 0..target_width {
                let x0 = u64::from(x) * u64::from(width) / u64::from(target_width);
                let x1 = u64::from(x + 1) * u64::from(width) / u64::from(target_width);
                let mut sum = [0u64; 4];
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let at = ((sy * u64::from(width) + sx) * 4) as usize;
                        for c in 0..4 {
                            sum[c] += u64::from(src[at + c]);
                        }
                    }
                }
                let n = (x1 - x0) * (y1 - y0);
                out.extend(sum.map(|value| ((value + n / 2) / n) as u16));
            }
        }
        (out, target_width, MAX_HEIGHT)
    }
    if std::fs::read(path)? != bytes {
        return Err(io::Error::other("manual page changed during resizing"));
    }
    slot_store::atomic_write(path, &output)?;
    Ok(true)
}
