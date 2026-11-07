//! Fetches optional box art directly from ScreenScraper.
//!
//! This module is deliberately separate from `art::jobs`, which fetches cartridge labels.
//! The caller supplies ScreenScraper credentials and a root directory; this module only
//! discovers missing boxes, looks up ROMs, downloads usable box images, and stores them in the
//! layout consumed by `slot_store::box_art`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use slot_store::{atomic_write, box_art, scan, Cart, Platform};

const API: &str = "https://api.screenscraper.fr/api2/jeuInfos.php";
const SOFTNAME: &str = "slot-box";
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub developer_id: String,
    pub developer_password: String,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedBox {
    pub stem: String,
    pub platform: Platform,
    pub files: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Offline,
    Refused,
    Quota,
    BadResponse,
    Io,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchSummary {
    pub saved: Vec<SavedBox>,
    pub missing: Vec<String>,
}

/// Games for which the existing resolver cannot currently find a complete box.
pub fn missing(root: &Path) -> Vec<Cart> {
    scan(root).unwrap_or_default().into_iter()
        .filter(|cart| box_art(root, cart).is_none())
        .collect()
}

/// ScreenScraper system IDs for Slot's supported platforms.
pub fn system_id(platform: Platform) -> u32 {
    match platform {
        Platform::Gb => 9,
        Platform::Gbc => 10,
        Platform::Gba | Platform::Gbav => 12,
    }
}

fn escape(value: &str) -> String {
    value.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' =>
            (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

fn lookup_url(credentials: &Credentials, cart: &Cart) -> std::io::Result<String> {
    let crc = crate::art::rom::rom_crc(&cart.rom)?;
    let filename = cart.rom.file_name().unwrap_or_default().to_string_lossy();
    let params = [
        ("devid", credentials.developer_id.as_str()),
        ("devpassword", credentials.developer_password.as_str()),
        ("softname", SOFTNAME),
        ("output", "json"),
        ("ssid", credentials.username.as_str()),
        ("sspassword", credentials.password.as_str()),
        ("crc", ""),
        ("systemeid", ""),
        ("romtype", "rom"),
        ("romnom", filename.as_ref()),
        ("romtaille", ""),
    ];
    let mut query = Vec::new();
    for (key, value) in params {
        let value = match key {
            "crc" => format!("{crc:08X}"),
            "systemeid" => system_id(cart.platform).to_string(),
            "romtaille" => std::fs::metadata(&cart.rom)?.len().to_string(),
            _ => value.to_string(),
        };
        query.push(format!("{key}={}", escape(&value)));
    }
    Ok(format!("{API}?{}", query.join("&")))
}

/// Uses curl's config on stdin so the credential-bearing URL is not exposed in process arguments.
fn get(url: &str) -> Result<(u16, Vec<u8>), FetchError> {
    let mut child = Command::new("curl")
        .args(["-s", "-L", "-m", "60", "-A", SOFTNAME, "-w", "\n%{http_code}", "-K", "-"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|_| FetchError::Offline)?;
    let config = format!("url = \"{}\"\n", url.replace('\\', "\\\\").replace('"', "\\\""));
    child.stdin.take().ok_or(FetchError::Offline)?
        .write_all(config.as_bytes()).map_err(|_| FetchError::Offline)?;
    let output = child.wait_with_output().map_err(|_| FetchError::Offline)?;
    let split = output.stdout.iter().rposition(|&b| b == b'\n').ok_or(FetchError::BadResponse)?;
    let status = std::str::from_utf8(&output.stdout[split + 1..]).ok()
        .and_then(|s| s.trim().parse::<u16>().ok()).ok_or(FetchError::BadResponse)?;
    if status == 0 { return Err(FetchError::Offline); }
    Ok((status, output.stdout[..split].to_vec()))
}

fn media_for(value: &serde_json::Value) -> Option<(&'static str, String)> {
    let medias = value.get("response")?.get("jeu")?.get("medias")?.as_array()?;
    // ScreenScraper defines box-2D as the front, box-2D-side as the spine,
    // box-2D-back as the rear, and box-texture as the combined flat texture.
    let mut faces: [Option<String>; 3] = [None, None, None];
    let mut texture = None;
    for media in medias {
        if media.get("parent").and_then(|v| v.as_str()).is_some_and(|p| p != "jeu") { continue; }
        let (Some(kind), Some(url)) = (
            media.get("type").and_then(|v| v.as_str()),
            media.get("url").and_then(|v| v.as_str()),
        ) else { continue; };
        let url = url.to_string();
        match kind {
            "box-2D" => faces[0] = Some(url),
            "box-2D-side" => faces[1] = Some(url),
            "box-2D-back" => faces[2] = Some(url),
            "box-texture" => texture = Some(url),
            _ => {}
        }
    }
    if faces.iter().all(Option::is_some) { return Some(("faces", faces.into_iter().flatten().collect::<Vec<_>>().join("\n"))); }
    texture.map(|u| ("texture", u))
}

fn valid_png(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_IMAGE_BYTES
        && png::Decoder::new(std::io::Cursor::new(bytes)).read_info()
            .is_ok_and(|r| r.info().width > 0 && r.info().height > 0)
}

fn media_url(url: &str) -> String {
    format!("{url}&maxwidth=1200&outputformat=png")
}

/// Fetches each missing game's box media sequentially. Full face sets and texture images are
/// saved in resolver-compatible names; a front-only ScreenScraper result is not treated as a
/// complete box and is reported missing.
pub fn fetch_missing(root: &Path, credentials: &Credentials) -> Result<FetchSummary, FetchError> {
    let mut summary = FetchSummary::default();
    for cart in missing(root) {
        let url = lookup_url(credentials, &cart).map_err(|_| FetchError::Io)?;
        let (status, body) = get(&url)?;
        if status == 401 || status == 403 { return Err(FetchError::Refused); }
        if status == 430 { return Err(FetchError::Quota); }
        if status != 200 { summary.missing.push(cart.stem); continue; }
        let value: serde_json::Value = serde_json::from_slice(&body).map_err(|_| FetchError::BadResponse)?;
        let Some((kind, urls)) = media_for(&value) else {
            summary.missing.push(cart.stem);
            continue;
        };
        let sources: Vec<String> = if kind == "faces" { urls.lines().map(str::to_string).collect() } else { vec![urls] };
        let mut images = Vec::new();
        for source in sources {
            let (status, bytes) = get(&media_url(&source))?;
            if status != 200 || !valid_png(&bytes) { images.clear(); break; }
            images.push(bytes);
        }
        let kinds: &[&str] = match kind {
            "faces" => &["front", "side", "back"],
            "texture" => &["texture"],
            _ => &[],
        };
        if images.is_empty() || images.len() != kinds.len() {
            summary.missing.push(cart.stem);
            continue;
        }
        let mut files = Vec::new();
        for (image, face) in images.iter().zip(kinds) {
            let path = slot_store::box_art_path(root, &cart, face);
            atomic_write(&path, image).map_err(|_| FetchError::Io)?;
            files.push(path);
        }
        if box_art(root, &cart).is_some() {
            summary.saved.push(SavedBox { stem: cart.stem, platform: cart.platform, files });
        } else {
            summary.missing.push(cart.stem);
        }
    }
    Ok(summary)
}


/// Progress notifications emitted by the background fetch worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerEvent {
    Started { total: usize },
    Finished(Result<FetchSummary, FetchError>),
}

/// A dedicated box-art worker. Cancellation is checked between games; an active HTTP request
/// is allowed to finish before the worker exits.
pub struct Worker {
    events: std::sync::mpsc::Receiver<WorkerEvent>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Worker {
    pub fn start(root: PathBuf, credentials: Credentials) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::channel;
        use std::sync::Arc;
        let (tx, events) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let total = missing(&root).len();
            let _ = tx.send(WorkerEvent::Started { total });
            if stop.load(Ordering::Relaxed) {
                let _ = tx.send(WorkerEvent::Finished(Ok(FetchSummary::default())));
                return;
            }
            let result = fetch_missing(&root, &credentials);
            let _ = tx.send(WorkerEvent::Finished(result));
        });
        Self { events, cancel }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn poll(&self) -> Vec<WorkerEvent> {
        self.events.try_iter().collect()
    }
}
