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
}

struct BoxFaceArt {
    front: CartFace,
    side: CartFace,
    back: CartFace,
    flap: CartFace,
    pivot: [f32; 2],
    front_center: [f32; 2],
    japanese: bool,
}

impl BoxFaces {
    pub fn spawn(root: PathBuf) -> Self {
        let queue: Shared = Arc::new((
            Mutex::new(JobQueue { jobs: VecDeque::new(), closed: false }),
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
        Self {
            queue,
            built,
            building: Building::default(),
            asked: Vec::new(),
            free: Vec::new(),
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

        let mut waiting: Vec<Key> = self.asked
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

    fn place(&mut self, app: &mut App, compositor: &mut Compositor, key: Key, art: Option<BoxFaceArt>) {
        self.asked.retain(|want| (want.shelf, want.index) != key);
        if !self.building.finish(key) {
            return;
        }
        let faces = art.map(|art| {
            let front = self.texture(compositor, &art.front);
            let side = self.texture(compositor, &art.side);
            let back = self.texture(compositor, &art.back);
            let flap = self.texture(compositor, &art.flap);
            BoxFaceTextures {
                front,
                side,
                back,
                flap: Some(flap),
                front_size: (art.front.w, art.front.h),
                side_size: (art.side.w, art.side.h),
                back_size: (art.back.w, art.back.h),
                japanese: art.japanese,
                pivot: art.pivot,
                front_center: art.front_center,
            }
        });
        if let Some(old) = app.set_box_faces(key.0, key.1, faces) {
            self.free.extend([old.front, old.side, old.back]);
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
    let mut faces = if art.source == BoxArtSource::Texture {
        load_texture_atlas(&art.back.path)
    } else {
        let separate = (
            slot_ui::decode_png(&art.front.path),
            slot_ui::decode_png(&art.side.path),
            slot_ui::decode_png(&art.back.path),
        );
        if let (Some(front), Some(side), Some(back)) = separate {
            assemble(front, side, back)
        } else {
            None
        }
    };

    if faces.is_none() {
        // A corrupt or unreadable separate set does not hide a usable atlas.
        let texture = box_art_file(root, cart, "texture")?;
        faces = load_texture_atlas(&texture);
    }

    let mut faces = faces?;
    faces.japanese = is_japanese_release(cart);
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
    (back, bw, bh): (Vec<u8>, u32, u32),
) -> Option<BoxFaceArt> {
    if fw == 0 || fh == 0 || sw == 0 || sh == 0 || bw == 0 || bh == 0 {
        return None;
    }
    let scale = fh as f32;
    let pivot_x = bw as f32 * scale / bh as f32 + sw as f32 * scale / sh as f32;
    let front_width = fw as f32 * scale / fh as f32;
    let flap = flap_face(&back, bw, bh)?;
    Some(BoxFaceArt {
        front: CartFace { rgba: front, w: fw, h: fh },
        side: CartFace { rgba: side, w: sw, h: sh },
        back: CartFace { rgba: back, w: bw, h: bh },
        flap,
        pivot: [pivot_x, fh as f32 / 2.0],
        front_center: [pivot_x + front_width / 2.0, fh as f32 / 2.0],
        japanese: false,
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
    let back = crop(&rgba, w, h, 0, face_w)?;
    let side = crop(&rgba, w, h, face_w, side_w)?;
    let front_x = face_w.checked_add(side_w)?;
    let front = crop(&rgba, w, h, front_x, face_w)?;
    let flap = flap_face(&back, face_w, h)?;
    Some(BoxFaceArt {
        front: CartFace { rgba: front, w: face_w, h },
        side: CartFace { rgba: side, w: side_w, h },
        back: CartFace { rgba: back, w: face_w, h },
        flap,
        pivot: [(face_w + side_w) as f32, h as f32 / 2.0],
        front_center: [(face_w + side_w) as f32 + face_w as f32 / 2.0, h as f32 / 2.0],
        japanese: false,
    })
}

fn flap_face(back: &[u8], width: u32, height: u32) -> Option<CartFace> {
    if width == 0 || height == 0 {
        return None;
    }
    let offset = (width as usize / 2).checked_mul(4)?;
    let colour = back.get(offset..offset + 3)?;
    let (mut rgba, w, h) = slot_ui::decode_png_bytes(include_bytes!("../assets/box_flap.png"))
        .or_else(|| {
            eprintln!("slot: box flap: could not decode embedded artwork");
            None
        })?;
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel[1] > 200 && pixel[0] < 40 && pixel[2] < 40 {
            pixel[..3].copy_from_slice(colour);
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
    fn separate_art_samples_the_back_not_the_spine() {
        let front = (vec![255; 4 * 3 * 4], 4, 3);
        let side = (vec![10; 2 * 3 * 4], 2, 3);
        let mut back = vec![0; 4 * 3 * 4];
        back[8..12].copy_from_slice(&[12, 34, 56, 255]);
        let art = assemble(front, side, (back, 4, 3)).unwrap();
        let offset = (10 * art.flap.w as usize + 350) * 4;
        assert_eq!(&art.flap.rgba[offset..offset + 3], &[12, 34, 56]);
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
            encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
        }
        let art = load_texture_atlas(&path).unwrap();
        let offset = (10 * art.flap.w as usize + 350) * 4;
        assert_eq!(&art.flap.rgba[offset..offset + 3], &[21, 43, 65]);
    }
}

fn crop(src: &[u8], sw: u32, sh: u32, x: u32, width: u32) -> Option<Vec<u8>> {
    if width == 0 || x.checked_add(width)? > sw || src.len() < (sw as usize).checked_mul(sh as usize)?.checked_mul(4)? {
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
