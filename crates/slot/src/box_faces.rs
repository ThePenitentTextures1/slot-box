use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use slot_gfx::{Compositor, TexId};
use slot_store::{box_art, box_art_file, BoxArtSource, Cart, TexturePanel};
use slot_ui::{BoxFaceTextures, CartFace};

use crate::app::{App, FaceWant};

const WORKERS: usize = 2;
type Key = (usize, usize);
type Built = (Key, CartFace, CartFace, CartFace, [f32; 2], [f32; 2]);

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
    pivot: [f32; 2],
    front_center: [f32; 2],
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
            BoxFaceTextures {
                front,
                side,
                back,
                front_size: (art.front.w, art.front.h),
                side_size: (art.side.w, art.side.h),
                back_size: (art.back.w, art.back.h),
                pivot: art.pivot,
                front_center: art.front_center,
            }
        });
        if let Some(old) = app.set_box_faces(key.0, key.1, faces) {
            self.free.extend([old.front, old.side, old.back]);
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
    if art.source == BoxArtSource::Texture {
        return load_texture_atlas(&art.back.path);
    }
    let separate = Some((
        slot_ui::decode_png(&art.front.path),
        slot_ui::decode_png(&art.side.path),
        slot_ui::decode_png(&art.back.path),
    ));
    if let Some((Some(front), Some(side), Some(back))) = separate {
        if let Some(art) = assemble(front, side, back) {
            return Some(art);
        }
    }

    // A corrupt or unreadable separate set does not hide a usable atlas.
    let texture = box_art_file(root, cart, "texture")?;
    load_texture_atlas(&texture)
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
    Some(BoxFaceArt {
        front: CartFace { rgba: front, w: fw, h: fh },
        side: CartFace { rgba: side, w: sw, h: sh },
        back: CartFace { rgba: back, w: bw, h: bh },
        pivot: [pivot_x, fh as f32 / 2.0],
        front_center: [pivot_x + front_width / 2.0, fh as f32 / 2.0],
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
    Some(BoxFaceArt {
        front: CartFace { rgba: front, w: face_w, h },
        side: CartFace { rgba: side, w: side_w, h },
        back: CartFace { rgba: back, w: face_w, h },
        pivot: [(face_w + side_w) as f32, h as f32 / 2.0],
        front_center: [(face_w + side_w) as f32 + face_w as f32 / 2.0, h as f32 / 2.0],
    })
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
