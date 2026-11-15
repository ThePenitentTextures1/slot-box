use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use slot_gfx::{Compositor, TexId};
use slot_store::{box_art, box_art_file, box_art_path, BoxArtSource, Cart};
use slot_ui::{BoxFaceTextures, CartFace};

use crate::app::{App, FaceWant};

const WORKERS: usize = 2;
const DETAIL_CACHE_SIZE: usize = 5;
type Key = (usize, usize);
type DetailJob = (Key, Cart, bool, bool, bool);
type DetailResult = (DetailJob, Option<CartFace>, Option<CartFace>);

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
    primed: bool,
    free: Vec<TexId>,
    details: Sender<DetailJob>,
    detailed: Receiver<DetailResult>,
    detail_pending: Option<DetailJob>,
    detail_dead: bool,
    detail_lru: VecDeque<(Key, Cart)>,
    failed_back: HashMap<Key, Cart>,
    failed_flap: HashMap<Key, Cart>,
    sources: HashMap<PathBuf, bool>,
    resize_jobs: Sender<Cart>,
    resized: Receiver<Cart>,
    resize_pending: Option<Cart>,
    resize_queue: Option<VecDeque<Cart>>,
    resize_done: HashSet<PathBuf>,
    resize_dead: bool,
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
        let (details, jobs) = mpsc::channel::<DetailJob>();
        let (results, detailed) = mpsc::channel();
        let detail_root = root.clone();
        if let Err(e) = thread::Builder::new()
            .name("slot-box-details".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let (_, ref cart, texture, back, flap) = job;
                    let result = load_details(&detail_root, cart, texture, back, flap);
                    if results.send((job, result.0, result.1)).is_err() {
                        break;
                    }
                }
            })
        {
            eprintln!("slot: box details: worker thread failed to start: {e}");
        }
        let (resize_jobs, jobs) = mpsc::channel::<Cart>();
        let (completed, resized) = mpsc::channel();
        let resize_root = root.clone();
        if let Err(e) = thread::Builder::new()
            .name("slot-box-resize".into())
            .spawn(move || {
                while let Ok(cart) = jobs.recv() {
                    resize_box_art(&resize_root, &cart);
                    if completed.send(cart).is_err() {
                        break;
                    }
                }
            })
        {
            eprintln!("slot: box resize: worker thread failed to start: {e}");
        }
        Self {
            queue,
            built,
            building: Building::default(),
            asked: Vec::new(),
            primed: false,
            free: Vec::new(),
            details,
            detailed,
            detail_pending: None,
            detail_dead: false,
            detail_lru: VecDeque::new(),
            failed_back: HashMap::new(),
            failed_flap: HashMap::new(),
            sources: HashMap::new(),
            resize_jobs,
            resized,
            resize_pending: None,
            resize_queue: None,
            resize_done: HashSet::new(),
            resize_dead: false,
        }
    }

    pub fn sync(&mut self, app: &mut App, compositor: &mut Compositor) {
        for tex in app.shed_box_faces() {
            compositor.update_texture(tex, 1, 1, &[0; 4]);
            self.free.push(tex);
        }
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
            .filter(|want| want.on_screen && !self.primed)
            .map(|want| (want.shelf, want.index))
            .collect();
        while !waiting.is_empty() {
            let Ok((key, art)) = self.built.recv() else {
                return;
            };
            waiting.retain(|pending| *pending != key);
            self.place(app, compositor, key, art);
        }
        self.primed = true;
        self.sync_details(app, compositor);
        self.sync_resize(app);
    }

    fn sync_resize(&mut self, app: &App) {
        loop {
            match self.resized.try_recv() {
                Ok(cart) => {
                    self.resize_done.insert(cart.rom);
                    self.resize_pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.resize_dead {
                        eprintln!("slot: box resize: worker stopped");
                    }
                    self.resize_dead = true;
                    break;
                }
            }
        }
        if !app.boxes_active() || self.resize_dead || self.resize_pending.is_some() {
            return;
        }
        if self.resize_queue.is_none() {
            self.resize_queue = Some(app.box_resize_carts().into());
        }
        let preferred = app
            .box_prefetch_wants()
            .into_iter()
            .filter_map(|(shelf, index)| app.face_cart(shelf, index))
            .find(|cart| !self.resize_done.contains(&cart.rom))
            .cloned();
        let queue = self.resize_queue.as_mut().expect("resize queue");
        let next = preferred.or_else(|| {
            while queue
                .front()
                .is_some_and(|cart| self.resize_done.contains(&cart.rom))
            {
                queue.pop_front();
            }
            queue.pop_front()
        });
        if let Some(cart) = next {
            if self.resize_jobs.send(cart.clone()).is_err() {
                eprintln!("slot: box resize: worker unavailable");
                self.resize_dead = true;
            } else {
                self.resize_pending = Some(cart);
            }
        }
    }

    fn sync_details(&mut self, app: &mut App, compositor: &mut Compositor) {
        self.sync_details_with(app, |texture, art| match texture {
            Some(tex) => {
                compositor.update_texture(tex, art.w, art.h, &art.rgba);
                tex
            }
            None => compositor.create_texture(art.w, art.h, &art.rgba),
        });
    }

    fn sync_details_with(
        &mut self,
        app: &mut App,
        mut upload: impl FnMut(Option<TexId>, CartFace) -> TexId,
    ) {
        loop {
            let (job, back_art, flap_art) = match self.detailed.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.detail_dead {
                        eprintln!("slot: box details: worker stopped");
                    }
                    self.detail_pending = None;
                    self.detail_dead = true;
                    break;
                }
            };
            self.detail_pending = None;
            let (key @ (shelf, index), cart, _, back, flap) = job;
            if app.face_cart(shelf, index) != Some(&cart) {
                continue;
            }
            let Some(mut faces) = app.box_faces_at(shelf, index) else {
                continue;
            };
            if back {
                if let Some(art) = back_art {
                    faces.back_size = (art.w, art.h);
                    faces.back = upload(self.free.pop(), art);
                    self.failed_back.remove(&key);
                } else {
                    eprintln!("slot: box back: could not load {}", cart.stem);
                    self.failed_back.insert(key, cart.clone());
                    faces.back_size = (0, 1);
                    app.box_back_unavailable(shelf, index);
                }
            }
            if flap {
                if let Some(art) = flap_art {
                    faces.flap = Some(upload(self.free.pop(), art));
                    self.failed_flap.remove(&key);
                } else {
                    eprintln!("slot: box flap: could not load {}", cart.stem);
                    self.failed_flap.insert(key, cart.clone());
                }
            }
            app.set_box_faces(shelf, index, Some(faces));
            self.detail_lru.retain(|(old, _)| *old != key);
            self.detail_lru.push_back((key, cart));
        }

        let wants = app.box_prefetch_wants();
        let selected = wants
            .first()
            .and_then(|&key| app.face_cart(key.0, key.1).cloned().map(|cart| (key, cart)));
        if let Some(selected) = selected.as_ref() {
            if let Some(at) = self.detail_lru.iter().position(|entry| entry == selected) {
                let entry = self.detail_lru.remove(at).expect("LRU entry");
                self.detail_lru.push_back(entry);
            }
        }
        while self.detail_lru.len() > DETAIL_CACHE_SIZE {
            let (key, cart) = self.detail_lru.pop_front().expect("LRU entry");
            self.failed_back.remove(&key);
            self.failed_flap.remove(&key);
            if app.face_cart(key.0, key.1) != Some(&cart) {
                continue;
            }
            if let Some(mut faces) = app.box_faces_at(key.0, key.1) {
                if faces.back != faces.front {
                    self.free.push(faces.back);
                    upload(Some(faces.back), empty_face());
                }
                if let Some(flap) = faces.flap.take() {
                    self.free.push(flap);
                    upload(Some(flap), empty_face());
                }
                faces.back = faces.front;
                faces.back_size = (0, 0);
                app.set_box_faces(key.0, key.1, Some(faces));
            }
        }
        let loading = app
            .box_detail_want()
            .is_some_and(|(shelf, index, back, flap)| {
                let key = (shelf, index);
                let Some(cart) = app.face_cart(shelf, index) else {
                    return false;
                };
                (back && self.failed_back.get(&key) != Some(cart))
                    || (flap && self.failed_flap.get(&key) != Some(cart))
            });
        app.set_box_detail_loading(loading && !self.detail_dead);
        if self.detail_pending.is_some() || self.detail_dead {
            return;
        }
        for key in wants {
            let Some(cart) = app.face_cart(key.0, key.1).cloned() else {
                continue;
            };
            let Some(faces) = app.box_faces_at(key.0, key.1) else {
                continue;
            };
            let back = faces.back == faces.front
                && faces.back_size != (0, 1)
                && self.failed_back.get(&key) != Some(&cart);
            let flap = faces.flap.is_none() && self.failed_flap.get(&key) != Some(&cart);
            if !back && !flap {
                continue;
            }
            let Some(&texture) = self.sources.get(&cart.rom) else {
                continue;
            };
            let job = (key, cart, texture, back, flap);
            if self.details.send(job.clone()).is_err() {
                eprintln!("slot: box details: worker unavailable");
                self.detail_dead = true;
                app.set_box_detail_loading(false);
                return;
            }
            self.detail_pending = Some(job);
            break;
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

fn empty_face() -> CartFace {
    CartFace {
        rgba: vec![0; 4],
        w: 1,
        h: 1,
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
        load_texture_atlas(&art.front.path)
    } else {
        let separate = (
            slot_ui::decode_png(&art.front.path),
            slot_ui::decode_png(&art.side.path),
        );
        if let (Some(front), Some(side)) = separate {
            assemble(limit_height(front), limit_height(side))
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
    assemble(
        limit_height((front, face_w, h)),
        limit_height((side, side_w, h)),
    )
}

fn limit_height((rgba, w, h): (Vec<u8>, u32, u32)) -> (Vec<u8>, u32, u32) {
    if h <= 420 {
        (rgba, w, h)
    } else {
        slot_ui::thumbnail(&rgba, w, h, u32::MAX, 420)
    }
}

fn resize_box_art(root: &Path, cart: &Cart) {
    for (suffix, width, height, atlas) in [
        ("front", u32::MAX, 420, false),
        ("side", u32::MAX, 420, false),
        ("back", 720, u32::MAX, false),
        ("texture", u32::MAX, 420, true),
    ] {
        if let Some(path) = box_art_file(root, cart, suffix) {
            if let Err(e) = crate::manual_resize::resize_image(&path, width, height, atlas) {
                eprintln!("slot: box resize {}: {e}", path.display());
            }
        }
    }
}

fn load_details(
    root: &Path,
    cart: &Cart,
    texture: bool,
    back: bool,
    flap: bool,
) -> (Option<CartFace>, Option<CartFace>) {
    let path = box_art_file(root, cart, if texture { "texture" } else { "back" });
    let cached = if flap {
        box_art_file(root, cart, "flap").and_then(|path| {
            let Some((rgba, w, h)) = slot_ui::decode_png(&path) else {
                eprintln!("slot: box flap: could not decode {}", path.display());
                return None;
            };
            if (w, h) != (1, 1) {
                eprintln!("slot: box flap: {} must be a 1x1 PNG", path.display());
                return None;
            }
            flap_colour(rgba[..3].try_into().ok()?)
        })
    } else {
        None
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
        let image = path
            .as_deref()
            .and_then(|path| decode(path, texture))
            .or_else(|| {
                if texture {
                    return None;
                }
                let atlas = box_art_file(root, cart, "texture")?;
                decode(&atlas, true)
            })
            .map(|image| {
                if image.w <= 720 {
                    image
                } else {
                    let (rgba, w, h) =
                        slot_ui::thumbnail(&image.rgba, image.w, image.h, 720, u32::MAX);
                    CartFace { rgba, w, h }
                }
            });
        let flap_art = if flap && cached.is_none() {
            image.as_ref().and_then(|image| {
                let color = back_colour(&image.rgba, image.w, image.h)?;
                save_flap_colour(root, cart, color);
                flap_colour(color)
            })
        } else {
            None
        };
        (image, cached.or(flap_art))
    } else if flap {
        let color = if cached.is_none() {
            path.as_deref()
                .and_then(|p| colour_from_first_row(p, texture))
                .or_else(|| {
                    if texture {
                        return None;
                    }
                    colour_from_first_row(&box_art_file(root, cart, "texture")?, true)
                })
        } else {
            None
        };
        let generated = color.and_then(|color| {
            save_flap_colour(root, cart, color);
            flap_colour(color)
        });
        (None, cached.or(generated))
    } else {
        (None, None)
    }
}

fn colour_from_first_row(path: &Path, texture: bool) -> Option<[u8; 3]> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    decoder.set_limits(png::Limits { bytes: 64 << 20 });
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if info.interlaced || info.exif_metadata.is_some() {
        let (rgba, w, h) = slot_ui::decode_png(path)?;
        let width = if texture { h } else { w };
        return back_colour(&rgba, width, h);
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
    Some(rgb)
}

fn back_colour(back: &[u8], width: u32, height: u32) -> Option<[u8; 3]> {
    if width == 0 || height == 0 {
        return None;
    }
    let offset = (width as usize / 2).checked_mul(4)?;
    back.get(offset..offset + 3)?.try_into().ok()
}

fn save_flap_colour(root: &Path, cart: &Cart, colour: [u8; 3]) {
    let path = box_art_path(root, cart, "flap");
    if box_art_file(root, cart, "flap").is_some() {
        return;
    }
    let mut bytes = Vec::new();
    let encoded = (|| -> Result<(), png::EncodingError> {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header()?.write_image_data(&colour)
    })();
    if let Err(e) = encoded {
        eprintln!("slot: box flap: encoding {}: {e}", path.display());
        return;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            if let Err(e) = file.write_all(&bytes) {
                eprintln!("slot: box flap: writing {}: {e}", path.display());
                drop(file);
                if let Err(e) = std::fs::remove_file(&path) {
                    eprintln!(
                        "slot: box flap: removing incomplete {}: {e}",
                        path.display()
                    );
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => eprintln!("slot: box flap: creating {}: {e}", path.display()),
    }
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
    use slot_input::Action;

    #[test]
    fn resize_sweep_prioritizes_selected_and_neighboring_games() {
        let dir = tempfile::tempdir().unwrap();
        crate::root::ensure(dir.path());
        std::fs::create_dir_all(dir.path().join("Games/GBA")).unwrap();
        for i in 0..4 {
            std::fs::write(
                dir.path().join(format!("Games/GBA/Game{i}.gba")),
                vec![0; 256],
            )
            .unwrap();
        }
        slot_store::write_carousel_view(dir.path(), slot_store::CarouselView::Boxes).unwrap();
        slot_store::write_slot_state(
            dir.path(),
            &slot_store::SlotState {
                clock_set: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut app = App::boot(dir.path());
        for want in app.box_face_wants() {
            let front = TexId::from_raw(want.index + 1);
            app.set_box_faces(
                want.shelf,
                want.index,
                Some(BoxFaceTextures {
                    front,
                    side: front,
                    back: front,
                    flap: None,
                    front_size: (100, 100),
                    side_size: (20, 100),
                    back_size: (0, 0),
                    japanese: false,
                    pivot: [120.0, 50.0],
                    front_center: [170.0, 50.0],
                }),
            );
        }
        let mut worker = BoxFaces::spawn(dir.path().to_path_buf());
        let (send_jobs, jobs) = mpsc::channel();
        let (completed, results) = mpsc::channel();
        worker.resize_jobs = send_jobs;
        worker.resized = results;
        worker.sync_resize(&app);
        let first = jobs.try_recv().unwrap();
        assert_eq!(first.stem, "Game0");
        app.apply(Action::ShelfRight);
        completed.send(first).unwrap();
        worker.sync_resize(&app);
        let second = jobs.try_recv().unwrap();
        assert_eq!(second.stem, "Game1");
        completed.send(second).unwrap();
        worker.sync_resize(&app);
        let third = jobs.try_recv().unwrap();
        assert_eq!(third.stem, "Game2");
        completed.send(third).unwrap();
        worker.sync_resize(&app);
        assert_eq!(jobs.try_recv().unwrap().stem, "Game3");
    }

    #[test]
    fn pending_details_do_not_block_and_lru_reprioritizes_new_selection() {
        let dir = tempfile::tempdir().unwrap();
        crate::root::ensure(dir.path());
        std::fs::create_dir_all(dir.path().join("Games/GBA")).unwrap();
        for i in 0..8 {
            std::fs::write(
                dir.path().join(format!("Games/GBA/Game{i}.gba")),
                vec![0; 256],
            )
            .unwrap();
        }
        slot_store::write_carousel_view(dir.path(), slot_store::CarouselView::Boxes).unwrap();
        slot_store::write_slot_state(
            dir.path(),
            &slot_store::SlotState {
                clock_set: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut app = App::boot(dir.path());
        let mut worker = BoxFaces::spawn(dir.path().to_path_buf());
        for want in app.box_face_wants() {
            let cart = app.face_cart(want.shelf, want.index).unwrap();
            worker.sources.insert(cart.rom.clone(), false);
            let front = TexId::from_raw(want.index + 1);
            app.set_box_faces(
                want.shelf,
                want.index,
                Some(BoxFaceTextures {
                    front,
                    side: front,
                    back: front,
                    flap: None,
                    front_size: (100, 100),
                    side_size: (20, 100),
                    back_size: (0, 0),
                    japanese: false,
                    pivot: [120.0, 50.0],
                    front_center: [170.0, 50.0],
                }),
            );
        }
        let (send_jobs, jobs) = mpsc::channel();
        let (results, received) = mpsc::channel();
        worker.details = send_jobs;
        worker.detailed = received;
        let mut next = 100;
        let mut upload = |old: Option<TexId>, _: CartFace| {
            old.unwrap_or_else(|| {
                next += 1;
                TexId::from_raw(next)
            })
        };
        worker.sync_details_with(&mut app, &mut upload);
        let first = jobs.try_recv().unwrap();
        assert_eq!(first.1.stem, "Game0");
        worker.sync_details_with(&mut app, &mut upload);
        assert!(
            jobs.try_recv().is_err(),
            "pending work must not be duplicated"
        );
        app.apply(Action::ShelfRight);
        let face = || CartFace {
            rgba: vec![255; 4],
            w: 1,
            h: 1,
        };
        results.send((first, Some(face()), Some(face()))).unwrap();
        worker.sync_details_with(&mut app, &mut upload);
        let mut job = jobs.try_recv().unwrap();
        assert_eq!(job.1.stem, "Game1", "selection must outrank old neighbors");
        for expected in 1..7 {
            assert_eq!(job.1.stem, format!("Game{expected}"));
            app.apply(Action::ShelfRight);
            results.send((job, Some(face()), Some(face()))).unwrap();
            worker.sync_details_with(&mut app, &mut upload);
            assert!(worker.detail_lru.len() <= DETAIL_CACHE_SIZE);
            job = jobs.try_recv().unwrap();
        }
        assert_eq!(job.1.stem, "Game7");
        assert!(worker
            .detail_lru
            .iter()
            .all(|(_, cart)| cart.stem != "Game0"));
        let selected = app.box_prefetch_wants()[0];
        let faces = app.box_faces_at(selected.0, selected.1).unwrap();
        assert_eq!(
            faces.back, faces.front,
            "the current pending game was not misassigned another back"
        );
    }

    #[test]
    fn box_art_is_resized_on_disk_and_atlas_panels_remain_usable() {
        let dir = tempfile::tempdir().unwrap();
        let boxes = dir.path().join("Boxes/GBA");
        std::fs::create_dir_all(&boxes).unwrap();
        let write = |name: &str, w: u32, h: u32, pixels: &[u8]| {
            let file = std::fs::File::create(boxes.join(name)).unwrap();
            let mut encoder = png::Encoder::new(file, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(pixels)
                .unwrap();
        };
        write("Game-front.png", 600, 840, &vec![255; 600 * 840 * 4]);
        write("Game-side.png", 80, 840, &vec![255; 80 * 840 * 4]);
        write("Game-back.png", 1440, 1000, &vec![255; 1440 * 1000 * 4]);
        let (w, h) = (1800, 840);
        let mut atlas = vec![255; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                atlas[i] = if x < h {
                    10
                } else if x < w - h {
                    20
                } else {
                    30
                };
            }
        }
        write("Game-texture.png", w as u32, h as u32, &atlas);
        let cart = Cart {
            platform: slot_store::Platform::Gba,
            stem: "Game".into(),
            rom: "Game.gba".into(),
            label: None,
            title: "Game".into(),
            code: String::new(),
            shell: None,
        };
        let art = build(dir.path(), &cart).unwrap();
        assert_eq!((art.front.w, art.front.h), (300, 420));
        assert_eq!((art.side.w, art.side.h), (40, 420));
        let (back, _) = load_details(dir.path(), &cart, false, true, false);
        assert_eq!((back.as_ref().unwrap().w, back.unwrap().h), (720, 500));
        let (_, width, height) = slot_ui::decode_png(&boxes.join("Game-front.png")).unwrap();
        assert_eq!(
            (width, height),
            (600, 840),
            "loading must not rewrite artwork on the rendering path"
        );
        resize_box_art(dir.path(), &cart);
        for (name, dimensions) in [
            ("front", (300, 420)),
            ("side", (40, 420)),
            ("back", (720, 500)),
            ("texture", (900, 420)),
        ] {
            let (_, width, height) =
                slot_ui::decode_png(&boxes.join(format!("Game-{name}.png"))).unwrap();
            assert_eq!((width, height), dimensions);
        }
        let art = load_texture_atlas(&boxes.join("Game-texture.png")).unwrap();
        assert_eq!(
            (art.front.w, art.front.h, art.side.w, art.side.h),
            (420, 420, 60, 420)
        );
        assert_eq!(art.front.rgba[0], 30);
        assert_eq!(art.side.rgba[0], 20);
        let (back, _) = load_details(dir.path(), &cart, true, true, false);
        assert_eq!(back.unwrap().rgba[0], 10);
        let original = std::fs::read(boxes.join("Game-front.png")).unwrap();
        resize_box_art(dir.path(), &cart);
        assert_eq!(
            std::fs::read(boxes.join("Game-front.png")).unwrap(),
            original
        );
    }

    #[test]
    fn flap_uses_the_top_center_back_pixel_and_keeps_cardboard_white() {
        let mut back = vec![0; 3 * 2 * 4];
        back[4..8].copy_from_slice(&[42, 87, 136, 255]);
        back[12..16].copy_from_slice(&[1, 2, 3, 255]);
        let flap = flap_colour(back_colour(&back, 3, 2).unwrap()).unwrap();
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
        let (pixel, w, h) = slot_ui::decode_png(&boxes.join("Game-flap.png")).unwrap();
        assert_eq!((w, h, &pixel[..3]), (1, 1, &[12, 34, 56][..]));
        let flap = flap.unwrap();
        let offset = (10 * flap.w as usize + 350) * 4;
        assert_eq!(&flap.rgba[offset..offset + 3], &[12, 34, 56]);
        let worker = BoxFaces::spawn(dir.path().to_path_buf());
        worker
            .details
            .send(((0, 0), cart.clone(), false, true, true))
            .unwrap();
        let (_, worker_back, worker_flap) = worker
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
        assert_eq!(&flap.unwrap().rgba[offset..offset + 3], &[12, 34, 56]);
        std::fs::remove_file(boxes.join("Game-flap.png")).unwrap();
        let (_, flap) = load_details(dir.path(), &cart, false, true, true);
        assert_eq!(&flap.unwrap().rgba[offset..offset + 3], &[21, 43, 65]);

        std::fs::remove_file(boxes.join("Game-back.png")).unwrap();
        std::fs::remove_file(boxes.join("Game-texture.png")).unwrap();
        let (back_art, flap) = load_details(dir.path(), &cart, false, true, true);
        assert!(back_art.is_none());
        assert_eq!(&flap.unwrap().rgba[offset..offset + 3], &[21, 43, 65]);

        std::fs::write(boxes.join("Game-flap.png"), b"broken").unwrap();
        write("Game-back.png", 4, 3, &back);
        let (_, flap) = load_details(dir.path(), &cart, false, false, true);
        assert!(flap.is_some());
        assert_eq!(
            std::fs::read(boxes.join("Game-flap.png")).unwrap(),
            b"broken"
        );
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
        let flap = flap_colour(colour_from_first_row(&path, true).unwrap()).unwrap();
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
