//! Background loading of the selected game's manual cover.
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

pub struct ManualCovers {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    results: mpsc::Receiver<(u64, Cart, Option<CartFace>)>,
    wanted: Option<Cart>,
    generation: u64,
    texture: Option<TexId>,
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
                    let cover = match load_cover(&root, &cart) {
                        Ok(cover) => cover,
                        Err(e) => {
                            eprintln!("slot: manual cover {}: {e}", cart.stem);
                            None
                        }
                    };
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
            texture: None,
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
            app.set_manual_cover(None);
            let (lock, wake) = &*self.queue;
            lock.lock().unwrap_or_else(|e| e.into_inner()).pending =
                want.map(|cart| (self.generation, cart));
            wake.notify_one();
        }
        while let Ok((generation, cart, cover)) = self.results.try_recv() {
            if generation != self.generation || self.wanted.as_ref() != Some(&cart) {
                continue;
            }
            if let Some(cover) = cover {
                let tex = upload(self.texture, cover);
                self.texture = Some(tex);
                app.set_manual_cover(Some((cart, tex)));
            }
        }
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
    let mut bytes = Vec::new();
    std::fs::File::open(&page.path)?
        .take(crate::manual_resize::MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > crate::manual_resize::MAX_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "manual cover exceeds the file size limit",
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
            "manual cover exceeds the pixel limit",
        ));
    }
    let (rgba, w, h) = slot_ui::decode_png_bytes(&bytes).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("could not decode {}", page.path.display()),
        )
    })?;
    let (rgba, w, h) = if h > crate::manual_resize::MAX_HEIGHT {
        let max_w = (u64::from(w) * u64::from(crate::manual_resize::MAX_HEIGHT))
            .div_ceil(u64::from(h)) as u32;
        slot_ui::thumbnail(&rgba, w, h, max_w, crate::manual_resize::MAX_HEIGHT)
    } else {
        (rgba, w, h)
    };
    Ok(Some(CartFace { rgba, w, h }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slot_input::{Action, Btn};
    use slot_store::{write_carousel_view, CarouselView};
    use slot_ui::{BoxFaceTextures, Draw};
    use std::time::{Duration, Instant};

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
        let until = Instant::now() + Duration::from_secs(5);
        let mut uploaded = false;
        while !uploaded && Instant::now() < until {
            covers.sync_with(&mut app, |_, face| {
                assert_eq!(&face.rgba[..4], &[220, 10, 30, 255]);
                uploaded = true;
                manual_tex
            });
            thread::sleep(Duration::from_millis(5));
        }
        assert!(uploaded, "cover worker did not load the PNG");
        app.apply(Action::GbaDown(Btn::A));
        app.apply(Action::GbaUp(Btn::A));
        app.update(1.78);
        app.apply(Action::GbaDown(Btn::Up));
        app.update(1.0);
        let mut draws = Vec::new();
        app.draw(&mut draws);
        assert!(draws
            .iter()
            .any(|draw| matches!(draw, Draw::Tex { tex, x, y, w, h, .. }
            if *tex == manual_tex && *x >= 0.0 && *y >= 0.0 && *w > 0.0 && *h > 0.0)));
        app.apply(Action::GbaDown(Btn::B));
        app.update(1.93);
        app.apply(Action::ShelfRight);
        assert_eq!(app.selected_stem(), Some("Other"));
        let until = Instant::now() + Duration::from_secs(5);
        let mut updated = false;
        while !updated && Instant::now() < until {
            covers.sync_with(&mut app, |previous, face| {
                assert_eq!(previous, Some(manual_tex));
                assert_eq!(&face.rgba[..4], &[10, 20, 220, 255]);
                updated = true;
                manual_tex
            });
            thread::sleep(Duration::from_millis(5));
        }
        assert!(updated, "switching games did not reuse the cover texture");
    }
}
