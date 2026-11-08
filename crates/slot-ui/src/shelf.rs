use slot_gfx::{Canvas, Draw, TexId, OUT_H};
use slot_store::{Cart, Platform};

use crate::board::{board_in, Placed};
use crate::cart::{cart_box, label_colour, label_rect, label_text, shelf_shape, CART_H, CART_W};
use crate::hud::Millis;
use crate::silhouette::GbShell;
use crate::slot_chrome::{draw_empty_slot, MOUTH_H};

const SIDE_SCALE: f32 = 0.78;
pub const BOX_VIEW_SCALE: f32 = 0.45;
const BOX_PIXELS_PER_MM: f32 = (360.0 / 60.0 + 212.0 / 35.0 + 312.0 / 57.0 + 337.0 / 65.5) / 4.0;
const SIDE_ALPHA: f32 = 1.0;

pub fn rest_y(h: f32) -> f32 {
    let y = if h > CART_H as f32 {
        (OUT_H as f32 - MOUTH_H - h) / 2.0
    } else {
        (OUT_H + CART_H) as f32 / 2.0 - h
    };
    // A Game Boy pak is 337px in a 422px gap, which centres at 42.5. Snap to a
    // whole pixel so the 312×337 bitmap hits the RG SP panel 1:1.
    y.round()
}

pub fn foot_y(h: f32) -> f32 {
    rest_y(h) + h
}

/// Returns physical front width/height and spine width at one shared Box View scale.
pub fn box_view_dimensions(cart: &Cart, japanese: bool) -> (f32, f32, f32) {
    let (front_width_mm, front_height_mm, depth_mm) = match cart.platform {
        Platform::Gba | Platform::Gbav if japanese => (90.0, 138.0, 24.0),
        Platform::Gba | Platform::Gbav => (125.0, 125.0, 24.0),
        Platform::Gb | Platform::Gbc if japanese => (100.0, 124.0, 22.0),
        Platform::Gb | Platform::Gbc => (125.0, 125.0, 28.0),
    };
    (
        front_width_mm * BOX_PIXELS_PER_MM,
        front_height_mm * BOX_PIXELS_PER_MM,
        depth_mm * BOX_PIXELS_PER_MM,
    )
}

fn loose_cart_dimensions(cart: &Cart) -> (f32, f32) {
    let (width_mm, height_mm) = match cart.platform {
        Platform::Gba | Platform::Gbav => (60.0, 35.0),
        Platform::Gb | Platform::Gbc => (57.0, 65.5),
    };
    (
        width_mm * BOX_PIXELS_PER_MM,
        height_mm * BOX_PIXELS_PER_MM,
    )
}

pub fn box_size(platform: Platform) -> (u32, u32) {
    match platform {
        Platform::Gba | Platform::Gbav => (318, 282),
        Platform::Gb | Platform::Gbc => (312, 277),
    }
}

const OMEGA: f32 = 16.0;
const PART: f32 = 130.0;

const SLOTS: i32 = 3;

pub const REPEAT_DELAY_MS: Millis = 400;

pub const HEART_SCALE: u32 = 2;
pub const HEART_INK: [u8; 3] = [0xe8, 0x3a, 0x3f];
const HEART_INSET: f32 = 5.0;
const REPEAT_MS: [Millis; 4] = [110, 85, 65, 50];

pub fn repeat_gap(fired: usize) -> Millis {
    REPEAT_MS[fired.min(REPEAT_MS.len() - 1)]
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxFaceTextures {
    pub front: TexId,
    pub side: TexId,
    pub back: TexId,
    pub front_size: (u32, u32),
    pub side_size: (u32, u32),
    pub back_size: (u32, u32),
    /// Whether the ROM identifies a Japanese release.
    pub japanese: bool,
    /// Pivot at the spine/front seam in the flattened net's pixel coordinates.
    pub pivot: [f32; 2],
    /// Front-face center in the flattened net's pixel coordinates.
    pub front_center: [f32; 2],
}

pub struct Shelf {
    pub carts: Vec<Cart>,
    pub index: usize,
    pub scroll: f32,
    faces: Vec<Option<TexId>>,
    box_faces: Vec<Option<BoxFaceTextures>>,
    box_checked: Vec<bool>,
    box_view: bool,
    wrapped: Vec<Option<TexId>>,
    interiors: Vec<Option<TexId>>,
    shadow: Option<TexId>,
    gb_shadow: Option<TexId>,
    gbc_shadow: Option<TexId>,
    backing: Option<TexId>,
    gb_backing: Option<TexId>,
    gbc_backing: Option<TexId>,
    hearted: Vec<bool>,
    heart: Option<(TexId, f32, f32)>,
    ride: f32,
    vel: f32,
    held: Option<(i32, Millis, usize)>,
    /// Tracks scroll position used exclusively for width interpolation.
    /// Lags behind `index` changes so that item widths transition smoothly
    /// as the spring carries the carousel from one stop to the next.
    width_scroll: f32,
    width_vel: f32,
}

impl Shelf {
    pub fn new(carts: Vec<Cart>) -> Self {
        Shelf {
            faces: vec![None; carts.len()],
            box_faces: vec![None; carts.len()],
            box_checked: vec![false; carts.len()],
            box_view: false,
            wrapped: vec![None; carts.len()],
            interiors: vec![None; carts.len()],
            hearted: vec![false; carts.len()],
            heart: None,
            carts,
            index: 0,
            scroll: 0.0,
            shadow: None,
            gb_shadow: None,
            gbc_shadow: None,
            backing: None,
            gb_backing: None,
            gbc_backing: None,
            ride: 0.0,
            vel: 0.0,
            held: None,
            width_scroll: 0.0,
            width_vel: 0.0,
        }
    }

    fn shell(&self, i: usize) -> Option<GbShell> {
        self.carts.get(i).and_then(|c| shelf_shape(c).0)
    }

    fn clear(&self, i: usize) -> bool {
        self.carts.get(i).is_some_and(|c| shelf_shape(c).1)
    }

    pub fn restock(&mut self, carts: Vec<Cart>) -> Vec<TexId> {
        let old: Vec<TexId> = self
            .faces
            .drain(..)
            .chain(self.interiors.drain(..))
            .flatten()
            .chain(self.box_faces.drain(..).flatten().flat_map(|b| [b.front, b.side, b.back]))
            .collect();
        self.faces = vec![None; carts.len()];
        self.box_faces = vec![None; carts.len()];
        self.box_checked = vec![false; carts.len()];
        self.wrapped = vec![None; carts.len()];
        self.interiors = vec![None; carts.len()];
        self.hearted = vec![false; carts.len()];
        self.carts = carts;
        self.held = None;
        self.select(0);
        old
    }

    pub fn select(&mut self, i: usize) {
        self.index = i;
        self.scroll = i as f32;
        self.ride = i as f32;
        self.vel = 0.0;
        self.width_scroll = i as f32;
        self.width_vel = 0.0;
    }

    pub fn set_box_view(&mut self, enabled: bool) {
        self.box_view = enabled;
    }

    pub fn set_heart(&mut self, face: TexId, w: u32, h: u32) {
        self.heart = Some((face, w as f32, h as f32));
    }

    pub fn mark_hearted(&mut self, hearted: impl Fn(&Cart) -> bool) {
        self.hearted = self.carts.iter().map(hearted).collect();
    }

    pub fn set_shadow(&mut self, face: TexId) {
        self.shadow = Some(face);
    }

    pub fn set_gb_shadow(&mut self, shell: GbShell, face: TexId) {
        match shell {
            GbShell::Notched => self.gb_shadow = Some(face),
            GbShell::Rounded => self.gbc_shadow = Some(face),
        }
    }

    pub fn set_backing(&mut self, face: TexId) {
        self.backing = Some(face);
    }

    pub fn set_gb_backing(&mut self, shell: GbShell, face: TexId) {
        match shell {
            GbShell::Notched => self.gb_backing = Some(face),
            GbShell::Rounded => self.gbc_backing = Some(face),
        }
    }

    pub fn set_faces(&mut self, faces: Vec<TexId>) {
        let mut faces = faces.into_iter();
        for face in &mut self.faces {
            *face = faces.next();
        }
    }

    pub fn face(&self, i: usize) -> Option<TexId> {
        self.faces.get(i).copied().flatten()
    }

    pub fn set_wrapped(&mut self, i: usize, tex: Option<TexId>) {
        if let Some(slot) = self.wrapped.get_mut(i) {
            *slot = tex;
        }
    }

    pub fn wrapped(&self, i: usize) -> Option<TexId> {
        self.wrapped.get(i).copied().flatten()
    }

    pub fn set_face(&mut self, i: usize, tex: TexId) -> Option<TexId> {
        self.faces.get_mut(i).and_then(|f| f.replace(tex))
    }

    pub fn box_faces(&self, i: usize) -> Option<BoxFaceTextures> {
        self.box_faces.get(i).copied().flatten()
    }

    pub fn box_checked(&self, i: usize) -> bool {
        self.box_checked.get(i).copied().unwrap_or(true)
    }

    pub fn set_box_faces(
        &mut self,
        i: usize,
        faces: Option<BoxFaceTextures>,
    ) -> Option<BoxFaceTextures> {
        let checked = self.box_checked.get_mut(i)?;
        *checked = true;
        std::mem::replace(self.box_faces.get_mut(i)?, faces)
    }

    pub fn take_box_faces(&mut self, i: usize) -> Option<BoxFaceTextures> {
        if let Some(checked) = self.box_checked.get_mut(i) {
            *checked = false;
        }
        self.box_faces.get_mut(i).and_then(Option::take)
    }

    pub fn set_interior(&mut self, i: usize, tex: TexId) -> Option<TexId> {
        self.interiors.get_mut(i).and_then(|f| f.replace(tex))
    }

    pub fn take_interior(&mut self, i: usize) -> Option<TexId> {
        self.interiors.get_mut(i).and_then(Option::take)
    }

    pub fn take_face(&mut self, i: usize) -> Option<TexId> {
        self.faces.get_mut(i).and_then(Option::take)
    }

    pub fn on_screen(&self) -> Vec<usize> {
        self.around(SLOTS)
    }

    pub fn around(&self, radius: i32) -> Vec<usize> {
        let mut out = Vec::new();
        for step in 0..=radius {
            for off in [step, -step] {
                if let Some(i) = self.cart_at_offset(off) {
                    if !out.contains(&i) {
                        out.push(i);
                    }
                }
            }
        }
        out
    }

    /// What a see-through cart shows behind its front shell, back plastic first and then the board.
    /// The shelf draws these under the face. The slot has to as well, or the board vanishes
    /// as the cart goes in or comes out.
    pub fn behind(&self, stem: &str) -> [Option<TexId>; 2] {
        let Some(i) = self.carts.iter().position(|c| c.stem == stem) else {
            return [None, None];
        };
        if !self.clear(i) || self.face(i).is_none() {
            return [None, None];
        }
        let board = match self.shell(i) {
            None => self.backing,
            Some(GbShell::Notched) => self.gb_backing,
            Some(GbShell::Rounded) => self.gbc_backing,
        };
        [self.interiors.get(i).copied().flatten(), board]
    }

    pub fn find(&self, stem: &str) -> Option<(&Cart, Option<TexId>)> {
        let i = self.carts.iter().position(|c| c.stem == stem)?;
        Some((&self.carts[i], self.face(i)))
    }

    pub fn left(&mut self) {
        self.step(-1);
    }

    pub fn right(&mut self) {
        self.step(1);
    }

    pub fn jump_next_letter(&mut self) {
        self.jump(1);
    }

    pub fn jump_prev_letter(&mut self) {
        self.jump(-1);
    }

    fn start_of_letter(&self, from: usize) -> usize {
        let n = self.carts.len();
        let letter = slot_store::initial(&self.carts[from].stem);
        let mut at = from;
        for _ in 0..n {
            let before = (at as i32 - 1).rem_euclid(n as i32) as usize;
            if slot_store::initial(&self.carts[before].stem) != letter {
                break;
            }
            at = before;
        }
        at
    }

    fn jump(&mut self, dir: i32) {
        let n = self.carts.len();
        if n < 2 {
            return;
        }
        let wrap = |i: i32| i.rem_euclid(n as i32) as usize;
        let here = slot_store::initial(&self.carts[self.index].stem);
        if self
            .carts
            .iter()
            .all(|c| slot_store::initial(&c.stem) == here)
        {
            return;
        }
        let target = match dir > 0 {
            true => {
                let mut at = self.index;
                for _ in 0..n {
                    at = wrap(at as i32 + 1);
                    if slot_store::initial(&self.carts[at].stem) != here {
                        break;
                    }
                }
                at
            }
            false => {
                let start = self.start_of_letter(self.index);
                match start == self.index {
                    true => self.start_of_letter(wrap(start as i32 - 1)),
                    false => start,
                }
            }
        };
        let ahead = (target as i32 - self.index as i32).rem_euclid(n as i32);
        let delta = match dir > 0 {
            true => ahead,
            false => ahead - n as i32,
        };
        self.index = target;
        self.ride += delta as f32;
        let to = self.scroll_target();
        self.scroll = to - (to - self.scroll).clamp(-1.0, 1.0);
    }

    pub fn hold_left(&mut self, now: Millis) {
        self.hold(-1, now);
    }

    pub fn hold_right(&mut self, now: Millis) {
        self.hold(1, now);
    }

    fn hold(&mut self, by: i32, now: Millis) {
        self.step(by);
        self.held = Some((by, now + REPEAT_DELAY_MS, 0));
    }

    pub fn release_left(&mut self) {
        self.release(-1);
    }

    pub fn release_right(&mut self) {
        self.release(1);
    }

    fn release(&mut self, by: i32) {
        if matches!(self.held, Some((held, _, _)) if held == by) {
            self.held = None;
        }
    }

    pub fn release_hold(&mut self) {
        self.held = None;
    }

    pub fn tick(&mut self, now: Millis) {
        let Some((by, due, fired)) = self.held else {
            return;
        };
        if now < due {
            return;
        }
        self.step(by);
        self.held = Some((by, now + repeat_gap(fired), fired + 1));
    }

    fn step(&mut self, by: i32) {
        let n = self.carts.len();
        if n < 2 {
            return;
        }
        self.index = (self.index as i32 + by).rem_euclid(n as i32) as usize;
        self.ride += by as f32;
    }

    pub fn scroll_target(&self) -> f32 {
        let n = self.carts.len();
        if n == 0 {
            return 0.0;
        }
        let from = self.ride;
        let n = n as f32;
        from + (self.index as f32 - from + n / 2.0).rem_euclid(n) - n / 2.0
    }

    pub fn cart_at_offset(&self, off: i32) -> Option<usize> {
        let n = self.carts.len() as i32;
        if n == 0 {
            return None;
        }
        if n == 1 {
            return (off == 0).then_some(self.index);
        }
        Some((self.index as i32 + off).rem_euclid(n) as usize)
    }

    pub fn selected_shell(&self) -> Option<GbShell> {
        self.shell(self.index)
    }

    pub fn selected_at(&self, canvas: Canvas) -> (f32, f32) {
        // Cartridge insertion and the core picker retain vanilla cartridge geometry even
        // when the selected shelf item itself is rendered as a box.
        let width = self
            .cart_at_offset(0)
            .map(|i| match self.carts[i].platform {
                Platform::Gba | Platform::Gbav => CART_W,
                platform => cart_box(platform).0,
            })
            .unwrap_or(CART_W) as f32;
        let scale = shrink(self.scroll_target() - self.scroll);
        (self.centre(canvas, 0) - width * scale / 2.0, scale)
    }

    fn slot_width(&self, slot: i32) -> (f32, bool) {
        let Some(index) = self.cart_at_offset(slot) else {
            return (CART_W as f32, true);
        };
        let cart = &self.carts[index];
        if self.box_view {
            // Interpolate between the outgoing and incoming widths while the
            // carousel spring is in flight. `t` is 0.0 at the moment of the
            // step and 1.0 when width_scroll has fully settled on the target.
            let target = self.scroll_target();
            let raw_t = (target - self.width_scroll).abs();
            let t = (1.0 - raw_t).clamp(0.0, 1.0);

            let width_for = |idx: usize, is_selected: bool| -> (f32, bool) {
                let c = &self.carts[idx];
                if let Some(faces) = self.box_faces(idx) {
                    let (front_w, _, side_w) = box_view_dimensions(c, faces.japanese);
                    ((if is_selected { front_w } else { side_w }) * BOX_VIEW_SCALE, false)
                } else {
                    let width = if is_selected {
                        cart_box(c.platform).0 as f32
                    } else {
                        loose_cart_dimensions(c).0
                    };
                    let scale = if is_selected { 1.0 } else { BOX_VIEW_SCALE };
                    (width * scale, false)
                }
            };

            // The outgoing item is the one that *was* in this display slot
            // before the most recent step — approximated by looking one step
            // back along the direction of travel. For slots that don't change
            // identity (neither endpoint of the motion), prev == curr.
            let is_selected = slot == 0;
            let (curr_w, _) = width_for(index, is_selected);

            // Find the index this slot held just before the last step.
            let n = self.carts.len() as i32;
            let travel_dir = (target - self.width_scroll).signum() as i32;
            let prev_idx = if n > 1 && travel_dir != 0 {
                // The slot's previous occupant was shifted by travel_dir.
                let prev_offset = slot - travel_dir;
                self.cart_at_offset(prev_offset).unwrap_or(index)
            } else {
                index
            };
            let prev_is_selected = (slot - travel_dir) == 0;
            let (prev_w, _) = width_for(prev_idx, prev_is_selected);

            let blended = prev_w + (curr_w - prev_w) * t;
            return (blended, false);
        }
        match cart.platform {
            Platform::Gba | Platform::Gbav => (CART_W as f32, true),
            p => (cart_box(p).0 as f32, false),
        }
    }

    fn pitch(&self, a: i32, u: f32) -> f32 {
        let ((wa, gba_a), (wb, gba_b)) = (self.slot_width(a), self.slot_width(a + 1));
        let (sa, sb) = (shrink(a as f32 - u), shrink((a + 1) as f32 - u));
        let gap_of = if gba_a || gba_b { CART_W as f32 } else { wa };
        wa * sa / 2.0 + gap_of * (1.0 - (sa + sb) / 2.0) + wb * sb / 2.0
    }

    fn centre(&self, canvas: Canvas, slot: i32) -> f32 {
        let u = self.scroll - self.scroll_target();
        let a = u.floor() as i32;
        let mut c = canvas.w / 2.0 - (u - a as f32) * self.pitch(a, u);
        if slot >= a {
            for s in a..slot {
                c += self.pitch(s, u);
            }
        } else {
            for s in (slot..a).rev() {
                c -= self.pitch(s, u);
            }
        }
        c
    }

    pub fn update(&mut self, dt: f32) {
        let target = self.scroll_target();
        let off = self.scroll - target;
        let pull = self.vel + OMEGA * off;
        let decay = (-OMEGA * dt).exp();
        self.scroll = target + (off + pull * dt) * decay;
        self.vel = (self.vel - OMEGA * pull * dt) * decay;

        // Drive width_scroll with the same spring so item widths transition
        // at the same rate as the positional scroll.
        let woff = self.width_scroll - target;
        let wpull = self.width_vel + OMEGA * woff;
        self.width_scroll = target + (woff + wpull * dt) * decay;
        self.width_vel = (self.width_vel - OMEGA * wpull * dt) * decay;
    }

    pub fn draw(
        &self,
        canvas: Canvas,
        shake: f32,
        slot: Option<(TexId, TexId)>,
        out: &mut Vec<Draw>,
    ) {
        self.draw_row(canvas, None, shake, 0.0, 1.0, out);
        draw_empty_slot(canvas, slot, out);
    }

    fn layers(&self, i: usize, rect: [f32; 4], alpha: f32, out: &mut Vec<Draw>) {
        let [x, y, w, h] = rect;
        if self.clear(i) && self.face(i).is_some() {
            let board = match self.shell(i) {
                None => self.backing,
                Some(GbShell::Notched) => self.gb_backing,
                Some(GbShell::Rounded) => self.gbc_backing,
            };
            let gba = self.shell(i).is_none();
            // A GBA cart's board and back shell fill the cart's box, exactly where the
            // picker lifts them from; a Game Boy pak's backing fills its box.
            let Placed {
                x: bx,
                y: by,
                w: bw,
                h: bh,
            } = if gba {
                board_in(Placed { x, y, w, h })
            } else {
                Placed { x, y, w, h }
            };
            // Back plastic, then the board, then the front shell.
            if let Some(tex) = self.interiors.get(i).copied().flatten() {
                out.push(Draw::Tex {
                    x: bx,
                    y: by,
                    w: bw,
                    h: bh,
                    tex,
                    alpha,
                });
            }
            if let Some(tex) = board {
                out.push(Draw::Tex {
                    x: bx,
                    y: by,
                    w: bw,
                    h: bh,
                    tex,
                    alpha,
                });
            }
        }
        out.push(match self.face(i) {
            Some(tex) => Draw::Tex {
                x,
                y,
                w,
                h,
                tex,
                alpha,
            },
            None => {
                let c = label_colour(&label_text(&self.carts[i]));
                Draw::Rect {
                    x,
                    y,
                    w,
                    h,
                    colour: [
                        c[0] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[2] as f32 / 255.0,
                        alpha,
                    ],
                }
            }
        });
    }

    pub fn draw_cart_at(&self, i: usize, rect: [f32; 4], out: &mut Vec<Draw>) {
        self.layers(i, rect, 1.0, out);
    }

    pub fn draw_row(
        &self,
        canvas: Canvas,
        hidden: Option<&str>,
        shake: f32,
        recede: f32,
        dim: f32,
        out: &mut Vec<Draw>,
    ) {
        let recede = recede.clamp(0.0, 1.0);
        let dim = dim.clamp(0.0, 1.0);
        let target = self.scroll_target();
        for slot in -SLOTS..=SLOTS {
            let Some(i) = self.cart_at_offset(slot) else {
                continue;
            };
            let cart = &self.carts[i];
            if hidden == Some(cart.stem.as_str()) {
                continue;
            }
            let offset = target + slot as f32 - self.scroll;
            let t = offset.abs().min(1.0);
            let scale = shrink(offset);
            let alpha = (1.0 + (SIDE_ALPHA - 1.0) * t) * (1.0 - recede);
            let (cw, ch) = cart_box(cart.platform);
            let box_faces = if self.box_view {
                self.box_faces(i)
            } else {
                None
            };
            let object_scale = if self.box_view && (box_faces.is_some() || slot != 0) {
                BOX_VIEW_SCALE * scale
            } else {
                scale
            };
            let (w, h) = if let Some(faces) = box_faces {
                let (front_w, front_h, side_w) = box_view_dimensions(cart, faces.japanese);
                if slot == 0 {
                    (front_w * object_scale, front_h * object_scale)
                } else {
                    (side_w * object_scale, front_h * object_scale)
                }
            } else if self.box_view && slot != 0 {
                let (width, height) = loose_cart_dimensions(cart);
                (width * object_scale, height * object_scale)
            } else {
                (cw as f32 * object_scale, ch as f32 * object_scale)
            };
            let away = offset.signum() * (1.0 + offset.abs());
            let x = self.centre(canvas, slot) - w / 2.0 + away * PART * recede;
            if x + w <= 0.0 || x >= canvas.w || alpha <= 0.0 {
                continue;
            }
            let x = x + shake;
            let y = if box_faces.is_some() {
                (OUT_H as f32 - h) / 2.0
            } else {
                foot_y(ch as f32) - h
            };
            if let Some(tex) = self.wrapped(i) {
                let (bw, bh) = box_size(cart.platform);
                let (bw, bh) = (bw as f32 * scale, bh as f32 * scale);
                out.push(Draw::Tex {
                    x: x + (w - bw) / 2.0,
                    y: y + h - bh,
                    w: bw,
                    h: bh,
                    tex,
                    alpha: alpha * dim,
                });
                continue;
            }
            if let Some(box_faces) = box_faces {
                if slot == 0 && box_faces.front_size.0 > 0 && box_faces.front_size.1 > 0 {
                    out.push(Draw::Tex {
                        x,
                        y,
                        w,
                        h,
                        tex: box_faces.front,
                        alpha: alpha * dim,
                    });
                    continue;
                } else if slot != 0 && box_faces.side_size.0 > 0 && box_faces.side_size.1 > 0 {
                    out.push(Draw::Tex {
                        x,
                        y,
                        w,
                        h,
                        tex: box_faces.side,
                        alpha: alpha * dim,
                    });
                    continue;
                }
            }
            if alpha < 1.0 {
                let backing = match self.shell(i) {
                    None => self.shadow,
                    Some(GbShell::Notched) => self.gb_shadow,
                    Some(GbShell::Rounded) => self.gbc_shadow,
                };
                if let Some(tex) = backing {
                    out.push(Draw::Tex {
                        x,
                        y,
                        w,
                        h,
                        tex,
                        alpha: recede_alpha(alpha),
                    });
                }
            }
            self.layers(i, [x, y, w, h], alpha * dim, out);
            if let (Some(true), Some((tex, sw, sh))) = (self.hearted.get(i), self.heart) {
                let (lx, ly, lw, _) = label_rect(self.shell(i));
                out.push(Draw::Tex {
                    x: (x + (lx + lw) * w - sw - HEART_INSET).round(),
                    y: (y + ly * h + HEART_INSET).round(),
                    w: sw,
                    h: sh,
                    tex,
                    alpha: alpha * dim,
                });
            }
        }
    }
}

fn shrink(offset: f32) -> f32 {
    1.0 + (SIDE_SCALE - 1.0) * offset.abs().min(1.0)
}

fn recede_alpha(face_alpha: f32) -> f32 {
    (face_alpha / SIDE_ALPHA).clamp(0.0, 1.0)
}
