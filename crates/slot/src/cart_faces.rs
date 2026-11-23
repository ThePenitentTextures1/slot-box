use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use slot_gfx::{Compositor, TexId};
use slot_store::Cart;
use slot_ui::{cart_face_with, cart_interior, shows_interior, CartFace};

use crate::app::{App, FaceWant};
use crate::label_cache::label_art;

const WORKERS: usize = 2;
const BOX_CART_CACHE_SIZE: usize = 5;

type Key = (usize, usize);

/// A cart's face and, for a clear GBA cart, the game-coloured inside it is drawn over.
type Built = (Key, CartFace, Option<CartFace>);

#[derive(Default)]
struct Queue {
    jobs: VecDeque<(Key, Cart)>,
    closed: bool,
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

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

    fn finish(&mut self, key: Key, seated: Option<&Cart>) -> bool {
        let built = self.carts.remove(&key);
        !self.outdated.remove(&key) && built.as_ref() == seated
    }
}

pub struct CartFaces {
    queue: Shared,
    built: Receiver<Built>,
    building: Building,
    asked: Vec<FaceWant>,
    free: Vec<TexId>,
    box_lru: VecDeque<(Key, Cart)>,
    unavailable: bool,
}

impl CartFaces {
    pub fn spawn(root: PathBuf) -> Self {
        let queue: Shared = Arc::default();
        let (outbox, built) = mpsc::channel();
        for n in 0..WORKERS {
            let (queue, outbox, root) = (queue.clone(), outbox.clone(), root.clone());
            let spawned = thread::Builder::new()
                .name(format!("slot-carts-{n}"))
                .spawn(move || work(&queue, &outbox, &root));
            if let Err(e) = spawned {
                eprintln!("slot: cart faces: worker thread failed to start: {e}");
            }
        }
        CartFaces {
            queue,
            built,
            building: Building::default(),
            asked: Vec::new(),
            free: Vec::new(),
            box_lru: VecDeque::new(),
            unavailable: false,
        }
    }

    pub fn sync(&mut self, app: &mut App, compositor: &mut Compositor) {
        for tex in app.shed_faces() {
            compositor.update_texture(tex, 1, 1, &[0; 4]);
            self.free.push(tex);
        }
        loop {
            match self.built.try_recv() {
                Ok((key, face, interior)) => self.place(app, compositor, key, face, interior),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.unavailable {
                        eprintln!("slot: cart faces: workers stopped");
                    }
                    self.unavailable = true;
                    break;
                }
            }
        }
        let wants = app.face_wants();
        if wants != self.asked {
            self.requeue(app, &wants);
            self.asked = wants;
        }
        let mut waiting: Vec<Key> = self
            .asked
            .iter()
            .filter(|w| w.on_screen && !app.boxes_active())
            .map(|w| (w.shelf, w.index))
            .collect();
        while !waiting.is_empty() {
            let Ok((key, face, interior)) = self.built.recv() else {
                return;
            };
            waiting.retain(|k| *k != key);
            self.place(app, compositor, key, face, interior);
        }
        let selected = app
            .box_prefetch_wants()
            .first()
            .and_then(|&key| app.face_cart(key.0, key.1).cloned().map(|cart| (key, cart)));
        if let Some(entry) = selected.as_ref() {
            if let Some(at) = self.box_lru.iter().position(|old| old == entry) {
                let entry = self.box_lru.remove(at).expect("cart cache entry");
                self.box_lru.push_back(entry);
            }
        }
        while self.box_lru.len() > BOX_CART_CACHE_SIZE {
            let (key, cart) = self.box_lru.pop_front().expect("cart cache entry");
            if app.face_cart(key.0, key.1) == Some(&cart) {
                for tex in app.take_prefetched_cart_faces(key.0, key.1) {
                    compositor.update_texture(tex, 1, 1, &[0; 4]);
                    self.free.push(tex);
                }
            }
        }
        app.set_box_cart_loading(!self.unavailable && app.box_cart_is_loading());
    }

    pub fn wait_selected(&mut self, app: &mut App, compositor: &mut Compositor) {
        let Some(selected) = app.selected_box_key() else {
            return;
        };
        while self.building.carts.contains_key(&selected) {
            match self.built.recv() {
                Ok((key, face, interior)) => {
                    self.place(app, compositor, key, face, interior);
                    self.sync(app, compositor);
                }
                Err(_) => {
                    if !self.unavailable {
                        eprintln!("slot: cart faces: workers stopped");
                    }
                    self.unavailable = true;
                    break;
                }
            }
        }
        app.set_box_cart_loading(!self.unavailable && app.box_cart_is_loading());
    }

    fn requeue(&mut self, app: &App, wants: &[FaceWant]) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        for (key, _) in queue.jobs.drain(..) {
            self.building.unqueue(key);
        }
        for w in wants {
            let key = (w.shelf, w.index);
            if let Some(cart) = app.face_cart(w.shelf, w.index) {
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
        face: CartFace,
        interior: Option<CartFace>,
    ) {
        self.asked.retain(|w| (w.shelf, w.index) != key);
        let cart = self.building.carts.get(&key).cloned();
        if !self.building.finish(key, app.face_cart(key.0, key.1)) {
            return;
        }
        let tex = self.texture(compositor, &face);
        self.free.extend(app.set_cart_face(key.0, key.1, tex));
        if let Some(interior) = interior {
            let tex = self.texture(compositor, &interior);
            self.free.extend(app.set_cart_interior(key.0, key.1, tex));
        }
        if app.box_faces_at(key.0, key.1).is_some() {
            if let Some(cart) = cart {
                self.box_lru.retain(|(old, _)| *old != key);
                self.box_lru.push_back((key, cart));
            }
        }
    }
}

impl Drop for CartFaces {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        wake.notify_all();
    }
}

fn work(queue: &Shared, outbox: &Sender<Built>, root: &Path) {
    let (lock, wake) = &**queue;
    loop {
        let (key, cart) = {
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if q.closed {
                    return;
                }
                if let Some(job) = q.jobs.pop_front() {
                    break job;
                }
                q = wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        slot_store::header_index::verify(&cart.rom);
        let face = cart_face_with(&cart, label_art(root, &cart));
        let interior = shows_interior(&cart).then(|| cart_interior(&cart));
        if outbox.send((key, face, interior)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slot_store::Platform;

    fn cart(stem: &str) -> Cart {
        Cart {
            platform: Platform::Gba,
            stem: stem.into(),
            rom: "a.gba".into(),
            label: None,
            title: stem.into(),
            code: String::new(),
            shell: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn a_face_built_from_an_old_cart_is_thrown_away() {
        let mut b = Building::default();
        let old = cart("A");
        let mut new = old.clone();
        new.label = Some("new.png".into());
        assert!(b.start((0, 0), &old));
        assert!(!b.start((0, 0), &new));
        assert!(!b.finish((0, 0), Some(&new)));
        assert!(b.start((0, 0), &new));
        assert!(b.finish((0, 0), Some(&new)));
    }

    #[test]
    fn the_same_cart_asked_twice_builds_once_and_lands() {
        let mut b = Building::default();
        assert!(b.start((0, 0), &cart("A")));
        assert!(!b.start((0, 0), &cart("A")));
        assert!(b.finish((0, 0), Some(&cart("A"))));
    }

    #[test]
    fn a_face_whose_seat_now_holds_another_cart_is_thrown_away() {
        let mut b = Building::default();
        assert!(b.start((3, 0), &cart("A")));
        assert!(!b.finish((3, 0), Some(&cart("B"))));
        assert!(b.start((3, 1), &cart("A")));
        assert!(!b.finish((3, 1), None));
    }
}
