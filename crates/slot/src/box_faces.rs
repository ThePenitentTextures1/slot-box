use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use slot_gfx::{Compositor, TexId};
use slot_store::{box_art, box_art_file, BoxArtSource, Cart};
use slot_ui::{BoxFaceTextures, CartFace};

use crate::app::{App, FaceWant};

const WORKERS: usize = 2;
type Key = (usize, usize);

struct JobQueue {
    jobs: VecDeque<(Key, Cart)>,
    closed: bool,
}
type Shared = Arc<(Mutex<JobQueue>, Condvar)>;

#[derive(Default)]
struct Building {
    carts: HashMap<Key, Cart>,
    outdated: HashSet<Key>,
}
impl Building {
    fn start(&mut self, key: Key, cart: &Cart) -> bool {
        match self.carts.get(&key) {
            None => {
                self.carts.insert(key, cart.clone());
                true
            }
            Some(building) => {
                if building != cart {
                    self.outdated.insert(key);
                }
                false
            }
        }
    }
    fn unqueue(&mut self, key: Key) {
        self.carts.remove(&key);
        self.outdated.remove(&key);
    }
    fn finish(&mut self, key: Key) -> bool {
        self.carts.remove(&key);
        !self.outdated.remove(&key)
    }
}

/// Asynchronously decodes optional box art for visible and nearby shelf games.
///
/// Image decoding and texture-atlas cropping happen on worker threads. GPU texture
/// creation and shelf updates stay on the frontend thread.
pub struct BoxFaces {
    queue: Shared,
    built: Receiver<(Key, Option<BoxFaceArt>)>,
    building: Building,
    asked: Vec<FaceWant>,
    free: Vec<TexId>,
    details: Sender<(Cart, bool, bool, bool)>,
    detailed: Receiver<(Option<CartFace>, Option<CartFace>)>,
    failed_back: HashMap<Key, Cart>,
    failed_flap: HashMap<Key, Cart>,
    sources: HashMap<PathBuf, bool>,
}

struct BoxFaceArt {
    front: CartFace,
    side: CartFace,
    pivot: [f32; 2],
    front_center: [f32; 2],
    japanese: bool,
    texture: bool,
}

impl BoxFaces {
    pub fn spawn(root: PathBuf) -> Self {
        let queue: Shared = Arc::new((
            Mutex::new(JobQueue {
                jobs: VecDeque::new(),
                closed: false,
            }),
            Condvar::new(),
        ));
        let (outbox, built) = mpsc::channel();
        for n in 0..WORKERS {
            let (queue, outbox, root) = (queue.clone(), outbox.clone(), root.clone());
            let spawned = thread::Builder::new()
                .name(format!("slot-box-faces-{n}"))
                .spawn(move || work(&queue, &outbox, &root));
            if let Err(e) = spawned {
                eprintln!("slot: box faces: worker thread failed to start: {e}");
            }
        }
        let (details, jobs) = mpsc::channel::<(Cart, bool, bool, bool)>();
        let (results, detailed) = mpsc::channel();
        let detail_root = root.clone();
        if let Err(e) = thread::Builder::new()
            .name("slot-box-details".into())
            .spawn(move || {
                while let Ok((cart, texture, back, flap)) = jobs.recv() {
                    let result = load_details(&detail_root, &cart, texture, back, flap);
                    if results.send(result).is_err() {
                        break;
                    }
                }
            })
        {
            eprintln!("slot: box details: worker thread failed to start: {e}");
        }
        Self {
            queue,
            built,
            building: Building::default(),
            asked: Vec::new(),
            free: Vec::new(),
            details,
            detailed,
            failed_back: HashMap::new(),
            failed_flap: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    pub fn sync(&mut self, app: &mut App, compositor: &mut Compositor) {
        self.free.extend(app.shed_box_faces());
        while let Ok((key, art)) = self.built.try_recv() {
            self.place(app, compositor, key, art);
        }

        let wants = app.box_face_wants();
        if wants != self.asked {
            self.requeue(app, &wants);
            self.asked = wants;
        }

        let mut waiting: Vec<Key> = self
            .asked
            .iter()
            .filter(|want| want.on_screen)
            .map(|want| (want.shelf, want.index))
            .collect();
        while !waiting.is_empty() {
            let Ok((key, art)) = self.built.recv() else {
                return;
            };
            waiting.retain(|pending| *pending != key);
            self.place(app, compositor, key, art);
        }
        self.sync_details(app, compositor);
    }

    fn sync_details(&mut self, app: &mut App, compositor: &mut Compositor) {
        let Some((shelf, index, back, flap)) = app.box_detail_want() else {
            return;
        };
        let key = (shelf, index);
        let Some(cart) = app.face_cart(shelf, index).cloned() else {
            return;
        };
        let back = back && self.failed_back.get(&key) != Some(&cart);
        let flap = flap && self.failed_flap.get(&key) != Some(&cart);
        if !back && !flap {
            return;
        }
        let Some(&texture) = self.sources.get(&cart.rom) else {
            return;
        };
        if self
            .details
            .send((cart.clone(), texture, back, flap))
            .is_err()
        {
            eprintln!("slot: box details: worker unavailable");
            return;
        }
        let Ok((back_art, flap_art)) = self.detailed.recv() else {
            eprintln!("slot: box details: worker stopped");
            return;
        };
        let Some(mut faces) = app.box_faces_at(shelf, index) else {
            return;
        };
        if back {
            if let Some(art) = back_art {
                faces.back = self.texture(compositor, &art);
                faces.back_size = (art.w, art.h);
                self.failed_back.remove(&key);
            } else {
                eprintln!("slot: box back: could not load {}", cart.stem);
                self.failed_back.insert(key, cart.clone());
            }
        }
        if flap {
            if let Some(art) = flap_art {
                faces.flap = Some(self.texture(compositor, &art));
                self.failed_flap.remove(&key);
            } else {
                eprintln!("slot: box flap: could not load {}", cart.stem);
                self.failed_flap.insert(key, cart);
            }
        }
        app.set_box_faces(shelf, index, Some(faces));
    }

    fn requeue(&mut self, app: &App, wants: &[FaceWant]) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        for (key, _) in queue.jobs.drain(..) {
            self.building.unqueue(key);
        }
        for want in wants {
            let key = (want.shelf, want.index);
            if let Some(cart) = app.face_cart(want.shelf, want.index) {
                if self.building.start(key, cart) {
                    queue.jobs.push_back((key, cart.clone()));
                }
            }
        }
        wake.notify_all();
    }

    fn texture(&mut self, compositor: &mut Compositor, face: &CartFace) -> TexId {
        match self.free.pop() {
            Some(tex) => {
                compositor.update_texture(tex, face.w, face.h, &face.rgba);
                tex
            }
            None => compositor.create_texture(face.w, face.h, &face.rgba),
        }
    }

    fn place(
        &mut self,
        app: &mut App,
        compositor: &mut Compositor,
        key: Key,
        art: Option<BoxFaceArt>,
    ) {
        self.asked.retain(|want| (want.shelf, want.index) != key);
        let building_cart = self.building.carts.get(&key).cloned();
        let matches_cart = building_cart.as_ref() == app.face_cart(key.0, key.1);
        if !self.building.finish(key) || !matches_cart {
            return;
        }
        self.failed_back.remove(&key);
        self.failed_flap.remove(&key);
        let faces = art.map(|art| {
            if let Some(cart) = building_cart {
                self.sources.insert(cart.rom, art.texture);
            }
            let front = self.texture(compositor, &art.front);
            let side = self.texture(compositor, &art.side);
            BoxFaceTextures {
                front,
                side,
                back: front,
                flap: None,
                front_size: (art.front.w, art.front.h),
                side_size: (art.side.w, art.side.h),
                back_size: (0, 0),
                japanese: art.japanese,
                pivot: art.pivot,
                front_center: art.front_center,
            }
        });
        if let Some(old) = app.set_box_faces(key.0, key.1, faces) {
            self.free.extend([old.front, old.side]);
            if old.back != old.front {
                self.free.push(old.back);
            }
            self.free.extend(old.flap);
        }
    }
}

impl Drop for BoxFaces {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        wake.notify_all();
    }
}

fn work(queue: &Shared, outbox: &Sender<(Key, Option<BoxFaceArt>)>, root: &Path) {
    let (lock, wake) = &**queue;
    loop {
        let (key, cart) = {
            let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if queue.closed {
                    return;
                }
                if let Some(job) = queue.jobs.pop_front() {
                    break job;
                }
                queue = wake.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
        };
        let art = build(root, &cart);
        if outbox.send((key, art)).is_err() {
            return;
        }
    }
}

fn build(root: &Path, cart: &Cart) -> Option<BoxFaceArt> {
    let art = box_art(root, cart)?;
    let mut texture = art.source == BoxArtSource::Texture;
    let mut faces = if art.source == BoxArtSource::Texture {
        load_texture_atlas(&art.back.path)
    } else {
        let separate = (
            slot_ui::decode_png(&art.front.path),
            slot_ui::decode_png(&art.side.path),
        );
        if let (Some(front), Some(side)) = separate {
            assemble(front, side)
        } else {
            None
        }
    };

    if faces.is_none() {
        // A corrupt or unreadable separate set does not hide a usable atlas.
        let texture_path = box_art_file(root, cart, "texture")?;
        faces = load_texture_atlas(&texture_path);
        texture = true;
    }

    let mut faces = faces?;
    faces.japanese = is_japanese_release(cart);
    faces.texture = texture;
    Some(faces)
}

fn is_japanese_release(cart: &Cart) -> bool {
    if is_japanese_stem(&cart.stem) {
        return true;
    }
    match cart.platform {
        slot_store::Platform::Gb | slot_store::Platform::Gbc => {
            slot_store::header_index::gb_header(&cart.rom).is_some_and(|header| header.japan)
        }
        slot_store::Platform::Gba | slot_store::Platform::Gbav => {
            slot_store::header_index::gba_code(&cart.rom).ends_with('J')
        }
        #[cfg(feature = "nds")]
        slot_store::Platform::Nds => false,
    }
}

fn is_japanese_stem(stem: &str) -> bool {
    let lower = stem.to_ascii_lowercase();
    lower
        .split(['(', '['])
        .skip(1)
        .map(|part| part.split([')', ']']).next().unwrap_or("").trim())
        .any(|tag| {
            tag == "japan"
                || tag.starts_with("japan,")
                || tag == "japanese"
                || tag.starts_with("japanese,")
                || tag == "jpn"
                || tag.starts_with("jpn,")
                || tag == "jp"
                || tag.starts_with("jp,")
                || tag == "j"
                || tag.starts_with("j,")
        })
}

fn assemble(
    (front, fw, fh): (Vec<u8>, u32, u32),
    (side, sw, sh): (Vec<u8>, u32, u32),
) -> Option<BoxFaceArt> {
    if fw == 0 || fh == 0 || sw == 0 || sh == 0 {
        return None;
    }
    let scale = fh as f32;
    let pivot_x = fw as f32 + sw as f32 * scale / sh as f32;
    let front_width = fw as f32 * scale / fh as f32;
    Some(BoxFaceArt {
        front: CartFace {
            rgba: front,
            w: fw,
            h: fh,
        },
        side: CartFace {
            rgba: side,
            w: sw,
            h: sh,
        },
        pivot: [pivot_x, fh as f32 / 2.0],
        front_center: [pivot_x + front_width / 2.0, fh as f32 / 2.0],
        japanese: false,
        texture: false,
    })
}

fn load_texture_atlas(path: &Path) -> Option<BoxFaceArt> {
    let (rgba, w, h) = slot_ui::decode_png(path)?;
    // ScreenScraper's box texture is packed as square back, narrow spine,
    // square front. This keeps the crop proportional at any image resolution.
    let face_w = h;
    let side_w = w.checked_sub(face_w.checked_mul(2)?)?;
    if side_w == 0 {
        return None;
    }
    let side = crop(&rgba, w, h, face_w, side_w)?;
    let front_x = face_w.checked_add(side_w)?;
    let front = crop(&rgba, w, h, front_x, face_w)?;
    Some(BoxFaceArt {
        front: CartFace {
            rgba: front,
            w: face_w,
            h,
        },
        side: CartFace {
            rgba: side,
            w: side_w,
            h,
        },
        pivot: [(face_w + side_w) as f32, h as f32 / 2.0],
        front_center: [
            (face_w + side_w) as f32 + face_w as f32 / 2.0,
            h as f32 / 2.0,
        ],
        japanese: false,
        texture: true,
    })
}

fn load_details(
    root: &Path,
    cart: &Cart,
    texture: bool,
    back: bool,
    flap: bool,
) -> (Option<CartFace>, Option<CartFace>) {
    let Some(path) = box_art_file(root, cart, if texture { "texture" } else { "back" }) else {
        return (None, None);
    };
    if back {
        let decode = |path: &Path, texture: bool| {
            slot_ui::decode_png(path).and_then(|(rgba, w, h)| {
                if texture {
                    let pixels = crop(&rgba, w, h, 0, h)?;
                    Some(CartFace {
                        rgba: pixels,
                        w: h,
                        h,
                    })
                } else {
                    Some(CartFace { rgba, w, h })
                }
            })
        };
        let image = decode(&path, texture).or_else(|| {
            if texture {
                return None;
            }
            let atlas = box_art_file(root, cart, "texture")?;
            decode(&atlas, true)
        });
        let flap_art = if flap {
            image
                .as_ref()
                .and_then(|image| flap_face(&image.rgba, image.w, image.h))
        } else {
            None
        };
        (image, flap_art)
    } else if flap {
        let flap = flap_from_first_row(&path, texture).or_else(|| {
            if texture {
                return None;
            }
            flap_from_first_row(&box_art_file(root, cart, "texture")?, true)
        });
        (None, flap)
    } else {
        (None, None)
    }
}

fn flap_from_first_row(path: &Path, texture: bool) -> Option<CartFace> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    decoder.set_limits(png::Limits { bytes: 64 << 20 });
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if info.interlaced || info.exif_metadata.is_some() {
        let (rgba, w, h) = slot_ui::decode_png(path)?;
        let width = if texture { h } else { w };
        return flap_face(&rgba, width, h);
    }
    let width = if texture { info.height } else { info.width };
    if width == 0 || width > info.width {
        return None;
    }
    let color = reader.output_color_type().0;
    let row = reader.next_row().ok()??;
    let bytes = row.data();
    let x = width as usize / 2;
    let rgb = match color {
        png::ColorType::Rgb => bytes.get(x * 3..x * 3 + 3)?.try_into().ok()?,
        png::ColorType::Rgba => bytes.get(x * 4..x * 4 + 3)?.try_into().ok()?,
        png::ColorType::Grayscale => [*bytes.get(x)?; 3],
        png::ColorType::GrayscaleAlpha => [*bytes.get(x * 2)?; 3],
        png::ColorType::Indexed => return None,
    };
    flap_colour(rgb)
}

fn flap_face(back: &[u8], width: u32, height: u32) -> Option<CartFace> {
    if width == 0 || height == 0 {
        return None;
    }
    let offset = (width as usize / 2).checked_mul(4)?;
    let colour = back.get(offset..offset + 3)?.try_into().ok()?;
    flap_colour(colour)
}

fn flap_colour(colour: [u8; 3]) -> Option<CartFace> {
    let (mut rgba, w, h) = slot_ui::decode_png_bytes(include_bytes!("../assets/box_flap.png"))
        .or_else(|| {
            eprintln!("slot: box flap: could not decode embedded artwork");
            None
        })?;
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel[1] > 200 && pixel[0] < 40 && pixel[2] < 40 {
            pixel[..3].copy_from_slice(&colour);
        }
    }
    Some(CartFace { rgba, w, h })
}

#[cfg(test)]
mod flap_tests {
    use super::*;

    #[test]
    fn flap_uses_the_top_center_back_pixel_and_keeps_cardboard_white() {
        let mut back = vec![0; 3 * 2 * 4];
        back[4..8].copy_from_slice(&[42, 87, 136, 255]);
        back[12..16].copy_from_slice(&[1, 2, 3, 255]);
        let flap = flap_face(&back, 3, 2).unwrap();
        let pixel = |x: usize, y: usize| &flap.rgba[(y * flap.w as usize + x) * 4..][..4];
        assert_eq!(pixel(350, 10), &[42, 87, 136, 255]);
        assert_eq!(pixel(350, 150), &[255, 255, 255, 255]);
    }

    #[test]
    fn separate_art_defers_the_back_and_samples_it_when_opened() {
        let dir = tempfile::tempdir().unwrap();
        let boxes = dir.path().join("Boxes/GBA");
        std::fs::create_dir_all(&boxes).unwrap();
        let write = |name: &str, w, h, pixels: &[u8]| {
            let file = std::fs::File::create(boxes.join(name)).unwrap();
            let mut encoder = png::Encoder::new(file, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(pixels)
                .unwrap();
        };
        write("Game-front.png", 4, 3, &vec![255; 4 * 3 * 4]);
        write("Game-side.png", 2, 3, &vec![10; 2 * 3 * 4]);
        std::fs::write(boxes.join("Game-back.png"), b"not a PNG").unwrap();
        let cart = Cart {
            platform: slot_store::Platform::Gba,
            stem: "Game".into(),
            rom: dir.path().join("Game.gba"),
            label: None,
            title: "Game".into(),
            code: String::new(),
            shell: None,
        };
        assert!(build(dir.path(), &cart).is_some());
        assert!(load_details(dir.path(), &cart, false, true, true)
            .0
            .is_none());
        let mut back = vec![0; 4 * 3 * 4];
        back[8..12].copy_from_slice(&[12, 34, 56, 255]);
        write("Game-back.png", 4, 3, &back);
        let (back_art, flap) = load_details(dir.path(), &cart, false, true, true);
        assert_eq!(back_art.unwrap().rgba, back);
        let flap = flap.unwrap();
        let offset = (10 * flap.w as usize + 350) * 4;
        assert_eq!(&flap.rgba[offset..offset + 3], &[12, 34, 56]);
        let worker = BoxFaces::spawn(dir.path().to_path_buf());
        worker
            .details
            .send((cart.clone(), false, true, true))
            .unwrap();
        let (worker_back, worker_flap) = worker
            .detailed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(worker_back.unwrap().rgba, back);
        assert_eq!(
            &worker_flap.unwrap().rgba[offset..offset + 3],
            &[12, 34, 56]
        );
        let (unloaded, quick_flap) = load_details(dir.path(), &cart, false, false, true);
        assert!(unloaded.is_none());
        assert_eq!(&quick_flap.unwrap().rgba[offset..offset + 3], &[12, 34, 56]);

        std::fs::write(boxes.join("Game-back.png"), b"broken").unwrap();
        let mut atlas = vec![0; 7 * 3 * 4];
        atlas[4..8].copy_from_slice(&[21, 43, 65, 255]);
        write("Game-texture.png", 7, 3, &atlas);
        let (back_art, flap) = load_details(dir.path(), &cart, false, true, true);
        assert_eq!(&back_art.unwrap().rgba[4..7], &[21, 43, 65]);
        assert_eq!(&flap.unwrap().rgba[offset..offset + 3], &[21, 43, 65]);
    }

    #[test]
    fn atlas_samples_the_top_center_of_the_cropped_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("texture.png");
        let mut pixels = vec![0; 7 * 3 * 4];
        pixels[4..8].copy_from_slice(&[21, 43, 65, 255]);
        pixels[12..16].copy_from_slice(&[99, 99, 99, 255]);
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = png::Encoder::new(file, 7, 3);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        let flap = flap_from_first_row(&path, true).unwrap();
        let offset = (10 * flap.w as usize + 350) * 4;
        assert_eq!(&flap.rgba[offset..offset + 3], &[21, 43, 65]);
    }
}

fn crop(src: &[u8], sw: u32, sh: u32, x: u32, width: u32) -> Option<Vec<u8>> {
    if width == 0
        || x.checked_add(width)? > sw
        || src.len() < (sw as usize).checked_mul(sh as usize)?.checked_mul(4)?
    {
        return None;
    }
    let mut out = Vec::with_capacity((width as usize).checked_mul(sh as usize)?.checked_mul(4)?);
    for y in 0..sh as usize {
        let start = (y * sw as usize + x as usize) * 4;
        let end = start + width as usize * 4;
        out.extend_from_slice(src.get(start..end)?);
    }
    Some(out)
}
