//! Background loading of the selected game's manual cover and nearby pages.
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;

use slot_gfx::{Compositor, TexId};
use slot_store::Cart;
use slot_ui::CartFace;

use crate::app::App;

struct Queue {
    pending: Option<(u64, Cart)>,
    closed: bool,
}

type CoverResult = (
    u64,
    Cart,
    std::io::Result<(Vec<slot_store::ManualPage>, Option<CartFace>)>,
);
const COVER_CACHE_SIZE: usize = 5;

struct CachedCover {
    cart: Cart,
    pages: Vec<slot_store::ManualPage>,
    texture: Option<TexId>,
}

pub struct ManualCovers {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    results: mpsc::Receiver<CoverResult>,
    wanted: Option<Cart>,
    generation: u64,
    cache: VecDeque<CachedCover>,
    cover_pending: Option<Cart>,
    cover_unavailable: bool,
    applied: Option<Cart>,
    pages: Vec<slot_store::ManualPage>,
    page_worker: PageWorker,
    page_textures: Vec<(usize, TexId)>,
    free: Vec<TexId>,
    requested: Vec<usize>,
}

struct PageWorker {
    queue: Arc<(Mutex<PageQueue>, Condvar)>,
    results: mpsc::Receiver<(u64, usize, std::io::Result<CartFace>)>,
}

struct PageQueue {
    pending: std::collections::VecDeque<(u64, usize, PathBuf)>,
    closed: bool,
}

impl PageWorker {
    fn spawn() -> Self {
        let queue = Arc::new((
            Mutex::new(PageQueue {
                pending: std::collections::VecDeque::new(),
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker_queue = Arc::clone(&queue);
        let (sender, results) = mpsc::sync_channel(2);
        if let Err(e) = thread::Builder::new()
            .name("slot-manual-pages".into())
            .spawn(move || {
                let (lock, wake) = &*worker_queue;
                loop {
                    let (generation, index, path) = {
                        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while !queue.closed && queue.pending.is_empty() {
                            queue = wake.wait(queue).unwrap_or_else(|e| e.into_inner());
                        }
                        if queue.closed {
                            return;
                        }
                        queue.pending.pop_front().expect("pending manual page")
                    };
                    if sender.send((generation, index, load_page(&path))).is_err() {
                        return;
                    }
                }
            })
        {
            eprintln!("slot: manual pages: worker failed to start: {e}");
        }
        Self { queue, results }
    }
}

impl Drop for PageWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        wake.notify_all();
    }
}

impl ManualCovers {
    pub fn spawn(root: PathBuf) -> Self {
        let queue = Arc::new((
            Mutex::new(Queue {
                pending: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let (sender, results) = mpsc::channel();
        let worker_queue = Arc::clone(&queue);
        if let Err(e) = thread::Builder::new()
            .name("slot-manual-cover".into())
            .spawn(move || {
                let (lock, wake) = &*worker_queue;
                loop {
                    let (generation, cart) = {
                        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while !queue.closed && queue.pending.is_none() {
                            queue = wake.wait(queue).unwrap_or_else(|e| e.into_inner());
                        }
                        if queue.closed {
                            return;
                        }
                        queue.pending.take().expect("pending manual cover")
                    };
                    let cover = slot_store::manual_pages(&root, &cart).and_then(|pages| {
                        let cover = pages.first().map(|p| load_page(&p.path)).transpose()?;
                        Ok((pages, cover))
                    });
                    if sender.send((generation, cart, cover)).is_err() {
                        return;
                    }
                }
            })
        {
            eprintln!("slot: manual cover: worker failed to start: {e}");
        }
        Self {
            queue,
            results,
            wanted: None,
            generation: 0,
            cache: VecDeque::new(),
            cover_pending: None,
            cover_unavailable: false,
            applied: None,
            pages: Vec::new(),
            page_worker: PageWorker::spawn(),
            page_textures: Vec::new(),
            free: Vec::new(),
            requested: Vec::new(),
        }
    }

    pub fn sync(&mut self, app: &mut App, compositor: &mut Compositor) {
        self.sync_with(app, |texture, cover| match texture {
            Some(tex) => {
                compositor.update_texture(tex, cover.w, cover.h, &cover.rgba);
                tex
            }
            None => compositor.create_texture(cover.w, cover.h, &cover.rgba),
        });
    }

    fn sync_with(
        &mut self,
        app: &mut App,
        mut upload: impl FnMut(Option<TexId>, CartFace) -> TexId,
    ) {
        let want = app.manual_cover_want();
        if want != self.wanted {
            self.generation = self.generation.wrapping_add(1);
            self.wanted = want.clone();
            self.applied = None;
            app.set_manual_cover(None);
            app.set_manual_page_count(0);
            app.set_manual_page_textures(Vec::new());
            self.pages.clear();
            self.requested.clear();
            self.free
                .extend(self.page_textures.drain(..).map(|(_, tex)| tex));
            self.page_worker
                .queue
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .clear();
            if let Some(cart) = want.as_ref() {
                if let Some(at) = self.cache.iter().position(|entry| &entry.cart == cart) {
                    let entry = self.cache.remove(at).expect("cover cache entry");
                    self.cache.push_back(entry);
                }
            }
        }
        loop {
            let (_generation, cart, result) = match self.results.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.cover_unavailable {
                        eprintln!("slot: manual cover: worker stopped");
                    }
                    self.cover_pending = None;
                    self.cover_unavailable = true;
                    break;
                }
            };
            self.cover_pending = None;
            let (pages, cover) = match result {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("slot: manual cover {}: {e}", cart.stem);
                    (Vec::new(), None)
                }
            };
            let texture = cover.map(|cover| upload(self.free.pop(), cover));
            self.cache.retain(|entry| entry.cart != cart);
            self.cache.push_back(CachedCover {
                cart,
                pages,
                texture,
            });
        }
        let selected = app
            .box_prefetch_wants()
            .first()
            .and_then(|&(shelf, index)| app.face_cart(shelf, index));
        if let Some(cart) = self.wanted.as_ref().or(selected) {
            if let Some(at) = self.cache.iter().position(|entry| &entry.cart == cart) {
                let entry = self.cache.remove(at).expect("cover cache entry");
                self.cache.push_back(entry);
            }
        }
        while self.cache.len() > COVER_CACHE_SIZE {
            let entry = self.cache.pop_front().expect("cover cache entry");
            if let Some(tex) = entry.texture {
                upload(
                    Some(tex),
                    CartFace {
                        rgba: vec![0; 4],
                        w: 1,
                        h: 1,
                    },
                );
                self.free.push(tex);
            }
        }
        if self.applied != self.wanted {
            if let Some(cart) = self.wanted.as_ref() {
                if let Some(entry) = self.cache.iter().find(|entry| &entry.cart == cart) {
                    self.pages = entry.pages.clone();
                    app.set_manual_page_count(self.pages.len());
                    app.set_manual_cover(entry.texture.map(|tex| (cart.clone(), tex)));
                    self.applied = Some(cart.clone());
                }
            }
        }
        if self.cover_pending.is_none() && !self.cover_unavailable {
            let mut candidates: Vec<Cart> = app
                .box_prefetch_wants()
                .into_iter()
                .filter_map(|(shelf, index)| app.face_cart(shelf, index).cloned())
                .collect();
            if let Some(cart) = self.wanted.clone() {
                candidates.retain(|other| other != &cart);
                candidates.insert(0, cart);
            }
            if let Some(cart) = candidates
                .into_iter()
                .find(|cart| !self.cache.iter().any(|entry| &entry.cart == cart))
            {
                let (lock, wake) = &*self.queue;
                lock.lock().unwrap_or_else(|e| e.into_inner()).pending =
                    Some((self.generation, cart.clone()));
                self.cover_pending = Some(cart);
                wake.notify_one();
            }
        }
        let wants = app.manual_page_wants();
        self.page_textures.retain(|(index, tex)| {
            if wants.contains(index) {
                true
            } else {
                self.free.push(*tex);
                false
            }
        });
        self.requested.retain(|i| wants.contains(i));
        {
            let (lock, wake) = &*self.page_worker.queue;
            let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
            queue.pending.retain(|(generation, index, _)| {
                *generation == self.generation && wants.contains(index)
            });
            for index in &wants {
                if !self.requested.contains(index)
                    && !self.page_textures.iter().any(|(i, _)| i == index)
                {
                    if let Some(page) = self.pages.get(*index) {
                        self.requested.push(*index);
                        queue
                            .pending
                            .push_back((self.generation, *index, page.path.clone()));
                    }
                }
            }
            wake.notify_one();
        }
        while let Ok((generation, index, result)) = self.page_worker.results.try_recv() {
            if generation != self.generation
                || !wants.contains(&index)
                || self.page_textures.iter().any(|(i, _)| *i == index)
            {
                continue;
            }
            match result {
                Ok(page) => {
                    let tex = upload(self.free.pop(), page);
                    self.page_textures.push((index, tex));
                }
                Err(e) => {
                    eprintln!("slot: manual page {}: {e}", index + 1);
                    app.manual_page_failed(index);
                }
            }
        }
        app.set_manual_page_textures(self.page_textures.clone());
    }
}

impl Drop for ManualCovers {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        wake.notify_all();
    }
}

/// Load the first numbered page; missing manuals are not an error.
pub fn load_cover(root: &Path, cart: &Cart) -> std::io::Result<Option<CartFace>> {
    let pages = slot_store::manual_pages(root, cart)?;
    let Some(page) = pages.first() else {
        return Ok(None);
    };
    load_page(&page.path).map(Some)
}

fn load_page(path: &Path) -> std::io::Result<CartFace> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(crate::manual_resize::MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > crate::manual_resize::MAX_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "manual page exceeds the file size limit",
        ));
    }
    let reader = png::Decoder::new(bytes.as_slice())
        .read_info()
        .map_err(std::io::Error::other)?;
    if u64::from(reader.info().width) * u64::from(reader.info().height)
        > crate::manual_resize::MAX_PIXELS
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "manual page exceeds the pixel limit",
        ));
    }
    let (rgba, w, h) = slot_ui::decode_png_bytes(&bytes).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("could not decode {}", path.display()),
        )
    })?;
    let (rgba, w, h) = if h > crate::manual_resize::MAX_HEIGHT {
        let max_w = (u64::from(w) * u64::from(crate::manual_resize::MAX_HEIGHT))
            .div_ceil(u64::from(h)) as u32;
        slot_ui::thumbnail(&rgba, w, h, max_w, crate::manual_resize::MAX_HEIGHT)
    } else {
        (rgba, w, h)
    };
    Ok(CartFace { rgba, w, h })
}

#[cfg(test)]
mod tests {
    use super::*;
    use slot_input::{Action, Btn};
    use slot_store::{write_carousel_view, CarouselView};
    use slot_ui::{BoxFaceTextures, Draw};
    use std::time::{Duration, Instant};

    #[test]
    fn cover_cache_evicts_least_recently_selected_and_releases_storage() {
        let root = tempfile::tempdir().unwrap();
        crate::root::ensure(root.path());
        std::fs::create_dir_all(root.path().join("Games/GBA")).unwrap();
        for i in 0..6 {
            std::fs::write(
                root.path().join(format!("Games/GBA/Game{i}.gba")),
                vec![0; 256],
            )
            .unwrap();
        }
        write_carousel_view(root.path(), CarouselView::Boxes).unwrap();
        slot_store::write_slot_state(
            root.path(),
            &slot_store::SlotState {
                clock_set: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut app = App::boot(root.path());
        let mut covers = ManualCovers::spawn(root.path().to_path_buf());
        for want in app.box_face_wants() {
            let cart = app.face_cart(want.shelf, want.index).unwrap().clone();
            let tex = TexId::from_raw(want.index + 1);
            covers.cache.push_back(CachedCover {
                cart,
                pages: Vec::new(),
                texture: Some(tex),
            });
            app.set_box_faces(
                want.shelf,
                want.index,
                Some(BoxFaceTextures {
                    front: tex,
                    side: tex,
                    back: tex,
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
        let mut cleared = Vec::new();
        covers.sync_with(&mut app, |texture, face| {
            assert_eq!((face.w, face.h), (1, 1));
            cleared.push(texture.unwrap());
            texture.unwrap()
        });
        assert_eq!(covers.cache.len(), COVER_CACHE_SIZE);
        assert_eq!(covers.cache.back().unwrap().cart.stem, "Game0");
        assert!(!covers.cache.iter().any(|entry| entry.cart.stem == "Game1"));
        assert_eq!(cleared.len(), 1);
    }

    #[test]
    fn background_cover_reaches_the_open_box_draw_commands() {
        let root = tempfile::tempdir().unwrap();
        crate::root::ensure(root.path());
        std::fs::create_dir_all(root.path().join("Games/GBA")).unwrap();
        std::fs::write(root.path().join("Games/GBA/Game.gba"), vec![0; 256]).unwrap();
        std::fs::write(root.path().join("Games/GBA/Other.gba"), vec![0; 256]).unwrap();
        for (name, colour) in [("Game", [220, 10, 30]), ("Other", [10, 20, 220])] {
            let dir = root.path().join("Manuals/GBA").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let file =
                std::fs::File::create(dir.join(format!("{name}_page_1_image_1.png"))).unwrap();
            let mut encoder = png::Encoder::new(file, 2, 2);
            encoder.set_color(png::ColorType::Rgb);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&colour.repeat(4)).unwrap();
            writer.finish().unwrap();
        }
        let manual_dir = root.path().join("Manuals/GBA/Game");
        let file = std::fs::File::create(manual_dir.join("Game_page_2_image_1.png")).unwrap();
        let mut encoder = png::Encoder::new(file, 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[20, 200, 30].repeat(4)).unwrap();
        writer.finish().unwrap();
        std::fs::write(manual_dir.join("Game_page_3_image_1.png"), b"broken").unwrap();
        write_carousel_view(root.path(), CarouselView::Boxes).unwrap();
        let state = slot_store::SlotState {
            clock_set: true,
            ..Default::default()
        };
        slot_store::write_slot_state(root.path(), &state).unwrap();
        let mut app = App::boot(root.path());
        let box_tex = TexId::from_raw(1);
        for want in app.box_face_wants() {
            app.set_box_faces(
                want.shelf,
                want.index,
                Some(BoxFaceTextures {
                    front: box_tex,
                    side: box_tex,
                    back: box_tex,
                    flap: None,
                    front_size: (100, 100),
                    side_size: (20, 100),
                    back_size: (100, 100),
                    japanese: false,
                    pivot: [120.0, 50.0],
                    front_center: [170.0, 50.0],
                }),
            );
        }
        let mut covers = ManualCovers::spawn(root.path().to_path_buf());
        let manual_tex = TexId::from_raw(2);
        assert!(app.manual_cover_want().is_none());
        covers.sync_with(&mut app, |_, _| panic!("cover loaded on carousel"));
        assert_eq!(covers.cover_pending.as_ref().unwrap().stem, "Game");
        let until = Instant::now() + Duration::from_secs(5);
        while covers.cache.len() < 2 && Instant::now() < until {
            covers.sync_with(&mut app, |_, face| match &face.rgba[..3] {
                [220, 10, 30] => manual_tex,
                [10, 20, 220] => TexId::from_raw(4),
                colour => panic!("unexpected cover {colour:?}"),
            });
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            covers.cache.len(),
            2,
            "carousel should prefetch both covers"
        );
        app.apply(Action::GbaDown(Btn::A));
        app.apply(Action::GbaUp(Btn::A));
        assert!(app.manual_cover_want().is_some());
        covers.sync_with(&mut app, |_, _| panic!("cached cover was reloaded"));
        app.update(1.78);
        app.apply(Action::GbaDown(Btn::Up));
        app.update(1.0);
        let mut draws = Vec::new();
        app.draw(&mut draws);
        assert!(draws
            .iter()
            .any(|draw| matches!(draw, Draw::Tex { tex, x, y, w, h, .. }
            if *tex == manual_tex && *x >= 0.0 && *y >= 0.0 && *w > 0.0 && *h > 0.0)));
        app.apply(Action::GbaDown(Btn::A));
        app.update(0.31);
        let page_tex = TexId::from_raw(3);
        let until = Instant::now() + Duration::from_secs(5);
        let mut page_uploaded = false;
        while (!page_uploaded || app.toast() != Some(slot_ui::Toast::ManualPageFailed))
            && Instant::now() < until
        {
            covers.sync_with(&mut app, |_, face| {
                assert_eq!(&face.rgba[..4], &[20, 200, 30, 255]);
                page_uploaded = true;
                page_tex
            });
            thread::sleep(Duration::from_millis(5));
        }
        assert!(page_uploaded, "page worker did not load the PNG");
        assert_eq!(app.toast(), Some(slot_ui::Toast::ManualPageFailed));
        app.apply(Action::GbaDown(Btn::Right));
        app.update(0.51);
        draws.clear();
        app.draw(&mut draws);
        assert!(draws
            .iter()
            .any(|draw| matches!(draw, Draw::Tex { tex, .. } if *tex == page_tex)));
        app.apply(Action::GbaDown(Btn::B));
        app.update(0.51);
        app.update(0.31);
        app.apply(Action::GbaDown(Btn::B));
        app.update(1.93);
        app.apply(Action::ShelfRight);
        assert_eq!(app.selected_stem(), Some("Other"));
        assert!(app.manual_cover_want().is_none());
        covers.sync_with(&mut app, |_, _| panic!("cover loaded on carousel"));
        app.apply(Action::GbaDown(Btn::A));
        app.apply(Action::GbaUp(Btn::A));
        covers.sync_with(&mut app, |_, _| panic!("neighbor cover was reloaded"));
        assert_eq!(covers.applied.as_ref().unwrap().stem, "Other");
    }
}
