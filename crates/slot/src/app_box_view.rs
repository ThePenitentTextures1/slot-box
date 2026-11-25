use super::*;

const AUTO_LINK_FILE: &str = "Config/auto_link_menu.txt";
const ADAPTER_FOCUS: f32 = 0.4;
const STACK_GAP: f32 = 12.0;

fn adapter_game(cart: &Cart) -> bool {
    if cart.platform != Platform::Gba {
        return false;
    }
    let Some((title, code)) = slot_store::header_title_code(&cart.rom) else {
        return false;
    };
    crate::link_kind::boxed_pokemon_adapter(&code, &title, slot_store::header_clean(&cart.rom))
}

fn auto_link_enabled(root: &std::path::Path, stem: &str) -> bool {
    slot_store::ini::value(root, AUTO_LINK_FILE, stem).as_deref() == Some("yes")
}

fn draw_flap(
    tex: Option<TexId>,
    center: [f32; 2],
    w: f32,
    h: f32,
    lid: f32,
    tab: f32,
    out: &mut Vec<Draw>,
) {
    let Some(tex) = tex.filter(|_| lid > 0.001) else {
        return;
    };
    let full_lid_h = w * (88.0 / 700.0);
    let lid_h = full_lid_h * lid;
    let hinge_y = center[1] - h / 2.0 - lid_h;
    out.push(Draw::Part {
        x: center[0] - w / 2.0,
        y: hinge_y,
        w,
        h: lid_h,
        tex,
        alpha: 1.0,
        uv: [0.0, 2.0 / 3.0, 1.0, 1.0 / 3.0],
    });
    let fold = (std::f32::consts::PI * tab).cos();
    if fold.abs() > 0.001 {
        let tab_h = if fold < 0.0 {
            -fold * lid_h
        } else {
            fold * full_lid_h
        };
        out.push(Draw::Part {
            x: center[0] - w / 2.0,
            y: if fold < 0.0 { hinge_y - tab_h } else { hinge_y },
            w,
            h: tab_h,
            tex,
            alpha: 1.0,
            uv: [
                0.0,
                if fold < 0.0 { 1.0 / 3.0 } else { 0.0 },
                1.0,
                1.0 / 3.0,
            ],
        });
    }
}

fn manual_page_size(canvas: slot_gfx::Canvas) -> (f32, f32) {
    let height = canvas.h - MOUTH_H;
    (height * 115.0 / 85.0, height)
}

fn manual_bounds(reader: &boxview::ManualReader, canvas: slot_gfx::Canvas) -> (f32, f32) {
    let columns = if reader.spread(reader.page)[1].is_some() {
        2.0
    } else {
        1.0
    };
    let (w, h) = manual_page_size(canvas);
    (
        ((columns * w * reader.zoom - canvas.w) / 2.0).max(0.0),
        ((h * reader.zoom - (canvas.h - MOUTH_H)) / 2.0).max(0.0),
    )
}

fn manual_spine(reader: &boxview::ManualReader, canvas: slot_gfx::Canvas) -> f32 {
    let width = manual_page_size(canvas).0 * reader.zoom;
    let offset = if reader.spread(reader.page)[1].is_some() {
        0.0
    } else if reader.page == 0 {
        -width / 2.0
    } else {
        width / 2.0
    };
    canvas.w / 2.0 + offset + reader.pan[0]
}

fn manual_turn(
    reader: &mut boxview::ManualReader,
    canvas: slot_gfx::Canvas,
    to: usize,
    put_down: bool,
) {
    let width = manual_page_size(canvas).0;
    let dest_spread = reader.spread(to);
    let forward = to > reader.page;
    let to_spine = if dest_spread[1].is_some() {
        if forward {
            width
        } else {
            canvas.w - width
        }
    } else if to == 0 {
        (canvas.w - width) / 2.0
    } else {
        (canvas.w + width) / 2.0
    };
    reader.turn = Some(boxview::ManualTurn {
        from: reader.page,
        to,
        elapsed: 0.0,
        from_spine: manual_spine(reader, canvas),
        to_spine,
        put_down,
    });
    reader.buttons = [false; 6];
    reader.last_tap = None;
}

impl App {
    fn box_art_loading(&self) -> bool {
        self.box_detail_loading
            || self.box_cart_loading
            || (self.box_manual_loading && matches!(self.box_scene, BoxScene::Opening { .. }))
    }

    pub(super) fn box_footer_alphas(&self) -> (f32, f32) {
        let fade_in = |started: Millis| {
            (self.now().saturating_sub(started) as f32 / BOX_BAR_FADE_MS as f32).clamp(0.0, 1.0)
        };
        let open = if self.box_inspection_from_open {
            1.0
        } else {
            0.0
        };
        match self.box_scene {
            BoxScene::Opening { started } | BoxScene::QuickOpening { started } => {
                (fade_in(started), 0.0)
            }
            BoxScene::Open { .. } => (1.0, self.manual_reader.as_ref().map_or(0.0, |r| r.pickup)),
            BoxScene::PreparingClose { .. } => (1.0, 0.0),
            BoxScene::Closing { started } => (1.0 - fade_in(started + BOX_CLOSE_MS), 0.0),
            BoxScene::QuickClosing { started } => (1.0 - fade_in(started + BOX_QUICK_MS), 0.0),
            BoxScene::FlippingBack { started, .. } => (open, fade_in(started)),
            BoxScene::Back { .. } => (open, 1.0),
            BoxScene::FlippingFront { started, .. } => (open, 1.0 - fade_in(started)),
            BoxScene::Carousel | BoxScene::QuickLaunched => (0.0, 0.0),
        }
    }

    pub(super) fn apply_carousel_view(&mut self) {
        let enabled = self.carousel_view == CarouselView::Boxes;
        for (_, shelf) in &mut self.shelves {
            shelf.set_box_view(enabled);
        }
    }

    pub(super) fn selected_box(
        &self,
    ) -> Option<(&Cart, slot_ui::BoxFaceTextures, (f32, f32, f32))> {
        let shelf = self.shelf();
        let cart = shelf.carts.get(shelf.index)?;
        let faces = shelf.box_faces(shelf.index)?;
        let dimensions = slot_ui::box_view_dimensions(cart, faces.japanese);
        Some((cart, faces, dimensions))
    }

    pub fn manual_cover_want(&self) -> Option<Cart> {
        if self.carousel_view != CarouselView::Boxes
            || !(matches!(
                self.box_scene,
                BoxScene::Opening { .. }
                    | BoxScene::Open { .. }
                    | BoxScene::PreparingClose { .. }
                    | BoxScene::Closing { .. }
            ) || self.box_inspection_from_open
                && matches!(
                    self.box_scene,
                    BoxScene::FlippingBack { .. }
                        | BoxScene::Back { .. }
                        | BoxScene::FlippingFront { .. }
                ))
        {
            return None;
        }
        self.selected_box().map(|(cart, _, _)| cart.clone())
    }

    pub fn set_manual_cover(&mut self, cover: Option<(Cart, TexId)>) {
        self.manual_cover = cover;
    }

    pub fn set_manual_page_count(&mut self, count: usize) {
        self.manual_page_count = count;
    }

    pub fn set_manual_page_textures(&mut self, textures: Vec<(usize, TexId)>) {
        self.manual_page_textures = textures;
    }

    pub fn set_manual_page_sizes(&mut self, sizes: Vec<(usize, u32, u32)>) {
        self.manual_page_sizes = sizes;
    }

    fn inspection_scale(&self) -> f32 {
        let (_, _, (_, height, _)) = self.selected_box().expect("selected box");
        let panel_height = if self.panel.1 == 0 {
            self.canvas().h
        } else {
            self.panel.1 as f32
        };
        (420.0 * self.canvas().h / panel_height)
            .min(self.canvas().h - MOUTH_H)
            .min(self.canvas().w)
            / height
    }

    fn inspection_center(&self) -> [f32; 2] {
        [self.canvas().w / 2.0, (self.canvas().h - MOUTH_H) / 2.0]
    }

    fn start_box_inspection(&mut self, from_open: bool) {
        let (center, scale, angle) = if from_open {
            let (cart, _, (_, height, _)) = self.selected_box().expect("selected box");
            let canvas = self.canvas();
            let box_height = height * slot_ui::BOX_VIEW_SCALE;
            let (y, _) = self.open_box_positions(cart, box_height, 1.0, true);
            self.box_focus = 1.0;
            self.box_adapter_focus = 0.0;
            ([canvas.w / 2.0, y], slot_ui::BOX_VIEW_SCALE, 0.0)
        } else {
            self.shelf().selected_box_pose(self.canvas())
        };
        self.box_inspection_from_open = from_open;
        self.box_back = false;
        self.box_turn = 0.0;
        self.box_buttons = [false; 6];
        self.box_resetting = false;
        self.play_held = None;
        self.held_dir = None;
        self.shelf_mut().release_hold();
        self.box_scene = BoxScene::FlippingBack {
            started: self.now(),
            from: boxview::BoxPose {
                center,
                scale,
                angle,
            },
        };
    }

    pub fn manual_page_wants(&self) -> Vec<usize> {
        self.manual_reader
            .as_ref()
            .map_or_else(Vec::new, |reader| reader.wants())
    }

    pub fn manual_page_failed(&mut self, _index: usize) {
        self.hud.toast(Toast::ManualPageFailed, self.now());
    }

    pub(super) fn manual_page_texture(&self, index: usize) -> Option<TexId> {
        if index == 0 {
            return self.selected_manual_cover();
        }
        self.manual_page_textures
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, tex)| *tex)
    }

    pub(super) fn selected_manual_cover(&self) -> Option<TexId> {
        let (cart, tex) = self.manual_cover.as_ref()?;
        let (selected, _, _) = self.selected_box()?;
        (cart.platform == selected.platform && cart.stem == selected.stem).then_some(*tex)
    }

    pub(super) fn tick_box_scene(&mut self) {
        let now = self.now();
        if self.box_art_loading() {
            match self.box_scene {
                BoxScene::FlippingBack { from, .. } => {
                    self.box_scene = BoxScene::FlippingBack { started: now, from };
                    return;
                }
                BoxScene::Opening { .. } => {
                    self.box_scene = BoxScene::Opening { started: now };
                    return;
                }
                BoxScene::QuickOpening { .. } => {
                    self.box_scene = BoxScene::QuickOpening {
                        started: self.clock.round() as Millis,
                    };
                    return;
                }
                _ => {}
            }
        }
        if let BoxScene::PreparingClose { started, angle } = self.box_scene {
            let t = (now.saturating_sub(started) as f32 / BOX_FLIP_MS as f32).clamp(0.0, 1.0);
            self.box_turn = angle * (1.0 - t * t * (3.0 - 2.0 * t));
            if t >= 1.0 {
                self.box_turn = 0.0;
                self.box_scene = BoxScene::Closing {
                    started: started + BOX_FLIP_MS,
                };
            }
            return;
        }
        let quick_now = self.clock.round() as Millis;
        if matches!(self.box_scene, BoxScene::QuickOpening { started } if quick_now.saturating_sub(started) >= BOX_QUICK_MS)
        {
            self.box_scene = BoxScene::QuickLaunched;
            self.insert(self.box_quick_clean);
            return;
        }
        self.box_scene = match self.box_scene {
            BoxScene::FlippingBack { started, .. }
                if now.saturating_sub(started) >= BOX_FLIP_MS =>
            {
                self.box_turn = 0.0;
                self.box_back = false;
                self.box_buttons = [false; 6];
                self.box_resetting = false;
                self.box_pending = None;
                BoxScene::Back {
                    zoom: 1.0,
                    pan: [0.0, 0.0],
                }
            }
            BoxScene::FlippingFront { started, .. }
                if now.saturating_sub(started) >= BOX_FLIP_MS =>
            {
                self.box_back = false;
                self.box_turn = 0.0;
                if self.box_inspection_from_open {
                    BoxScene::Open { item: BoxItem::Box }
                } else {
                    BoxScene::Carousel
                }
            }
            BoxScene::Opening { started, .. }
                if now.saturating_sub(started) >= BOX_BAR_FADE_MS + BOX_OPEN_MS =>
            {
                BoxScene::Open {
                    item: BoxItem::Cartridge,
                }
            }
            BoxScene::Closing { started }
                if now.saturating_sub(started) >= BOX_CLOSE_MS + BOX_BAR_FADE_MS =>
            {
                BoxScene::Carousel
            }
            BoxScene::QuickClosing { started }
                if quick_now.saturating_sub(started) >= BOX_QUICK_MS + BOX_BAR_FADE_MS =>
            {
                BoxScene::Carousel
            }
            scene => scene,
        };
    }

    pub(super) fn start_box_open(&mut self, quick: bool, clean: bool) {
        self.box_adapter_present = self
            .selected_box()
            .is_some_and(|(cart, _, _)| adapter_game(cart));
        self.box_adapter_open = false;
        self.box_adapter_closing = false;
        self.box_adapter_pickup = 0.0;
        self.box_adapter_focus = 0.0;
        let (center, scale, angle) = self.shelf().selected_box_pose(self.canvas());
        self.box_open_from = boxview::BoxPose {
            center,
            scale,
            angle,
        };
        self.box_focus = 0.0;
        self.box_zoom = 1.0;
        self.box_pan = [0.0; 2];
        self.box_turn = 0.0;
        self.box_back = false;
        self.box_buttons = [false; 6];
        self.box_resetting = false;
        self.box_pending = None;
        self.box_quick_clean = clean;
        self.play_held = None;
        self.shelf_mut().release_hold();
        self.box_scene = if quick {
            BoxScene::QuickOpening {
                started: self.clock.round() as Millis,
            }
        } else {
            BoxScene::Opening {
                started: self.now(),
            }
        };
    }

    pub(super) fn tick_open_box(&mut self, dt: f32) {
        let canvas = self.canvas();
        let inspecting = matches!(self.box_scene, BoxScene::Back { .. });
        let item = match self.box_scene {
            BoxScene::Open { item } => item,
            BoxScene::Back { .. } => BoxItem::Box,
            _ => return,
        };
        if !inspecting {
            let target = match item {
                BoxItem::Manual => -1.0,
                BoxItem::Adapter => ADAPTER_FOCUS,
                BoxItem::Cartridge => 0.0,
                BoxItem::Box => 1.0,
            };
            let speed = if target < 0.0 || (target == 0.0 && self.box_focus < 0.0) {
                1.0 / 0.28
            } else {
                BOX_FOCUS_SPEED
            };
            let step = speed * dt.max(0.0);
            self.box_focus = if target > self.box_focus {
                (self.box_focus + step).min(target)
            } else {
                (self.box_focus - step).max(target)
            };
            if item == BoxItem::Cartridge && self.box_focus == 0.0 && self.on_shelf() {
                match self.box_pending.take() {
                    Some(BoxCartAction::Play(clean)) => self.insert(clean),
                    Some(BoxCartAction::Picker) => self.open_core_picker(),
                    Some(BoxCartAction::Studio) => self.open_studio(),
                    None => {}
                }
            }
        }
        let delta = dt.max(0.0) / boxview::MANUAL_PICKUP_SECONDS;
        self.box_adapter_focus = if item == BoxItem::Adapter {
            (self.box_adapter_focus + delta).min(1.0)
        } else {
            (self.box_adapter_focus - delta).max(0.0)
        };
        if self.box_adapter_open {
            self.box_adapter_pickup = if self.box_adapter_closing {
                (self.box_adapter_pickup - delta).max(0.0)
            } else {
                (self.box_adapter_pickup + delta).min(1.0)
            };
            if self.box_adapter_closing && self.box_adapter_pickup <= 0.0 {
                self.box_adapter_open = false;
                self.box_adapter_closing = false;
            }
        }
        let blend = 1.0 - (-12.0 * dt.max(0.0)).exp();
        let target_turn = if self.box_back {
            std::f32::consts::PI
        } else {
            0.0
        };
        self.box_turn += (target_turn - self.box_turn) * blend;
        if (self.box_turn - target_turn).abs() < 0.001 {
            self.box_turn = target_turn;
        }
        let manual_zoom_max = self
            .manual_reader
            .as_ref()
            .map(|reader| {
                let (width, height) = manual_page_size(canvas);
                let width = width
                    * if self.panel.0 == 0 {
                        1.0
                    } else {
                        self.panel.0 as f32 / canvas.w
                    };
                let height = height
                    * if self.panel.1 == 0 {
                        1.0
                    } else {
                        self.panel.1 as f32 / canvas.h
                    };
                reader
                    .spread(reader.page)
                    .into_iter()
                    .flatten()
                    .map(|index| {
                        self.manual_page_sizes
                            .iter()
                            .find(|(i, _, _)| *i == index)
                            .map_or(1.0, |(_, w, h)| (*h as f32 / height).min(*w as f32 / width))
                    })
                    .fold(2.0_f32, f32::min)
                    .clamp(1.0, 2.0)
            })
            .unwrap_or(1.0);
        if let Some(reader) = &mut self.manual_reader {
            if self.box_focus <= -0.999 {
                let delta = dt.max(0.0) / boxview::MANUAL_PICKUP_SECONDS;
                reader.pickup = if reader.closing {
                    (reader.pickup - delta).max(0.0)
                } else {
                    (reader.pickup + delta).min(1.0)
                };
            }
            let mut finished_turn = false;
            if let Some(turn) = &mut reader.turn {
                turn.elapsed += dt.max(0.0);
                if turn.elapsed >= boxview::MANUAL_TURN_SECONDS {
                    let to = turn.to;
                    let spine = turn.to_spine;
                    let put_down = turn.put_down;
                    reader.page = to;
                    reader.turn = None;
                    reader.pan[0] = 0.0;
                    reader.pan[0] = spine - manual_spine(reader, canvas);
                    reader.closing = put_down;
                    finished_turn = true;
                }
            }
            let blend = 1.0 - (-12.0 * dt.max(0.0)).exp();
            if reader.resetting {
                reader.zoom += (1.0 - reader.zoom) * blend;
                reader.pan[1] *= 1.0 - blend;
                if reader.zoom < 1.001 && reader.pan[1].abs() < 0.1 {
                    reader.zoom = 1.0;
                    reader.pan[1] = 0.0;
                    reader.resetting = false;
                }
            } else if reader.pickup >= 1.0 && !reader.closing && reader.turn.is_none() {
                let delta = reader.buttons[0] as i32 - reader.buttons[1] as i32;
                reader.zoom = (reader.zoom * (delta as f32 * dt.max(0.0) * 1.4).exp())
                    .clamp(1.0, manual_zoom_max);
                reader.pan[0] += (reader.buttons[2] as i32 - reader.buttons[3] as i32) as f32
                    * dt.max(0.0)
                    * 360.0;
                if reader.zoom > 1.001 {
                    reader.pan[1] += (reader.buttons[4] as i32 - reader.buttons[5] as i32) as f32
                        * dt.max(0.0)
                        * 240.0;
                }
            }
            let (max_x, max_y) = manual_bounds(reader, canvas);
            reader.pan[0] = reader.pan[0].clamp(-max_x, max_x);
            reader.pan[1] = reader.pan[1].clamp(-max_y, max_y);
            if reader.zoom <= 1.001 && !reader.resetting {
                reader.pan[1] = 0.0;
            }
            if !finished_turn && reader.closing && reader.pickup <= 0.0 && self.box_focus <= -0.999
            {
                self.manual_reader = None;
            }
        }
        if item != BoxItem::Box {
            return;
        }
        let (mut zoom, mut pan) = match self.box_scene {
            BoxScene::Back { zoom, pan } => (zoom, pan),
            _ => (self.box_zoom, self.box_pan),
        };
        if self.box_resetting {
            zoom += (1.0 - zoom) * blend;
            pan[0] *= 1.0 - blend;
            pan[1] *= 1.0 - blend;
            if zoom < 1.001 && pan[0].abs() < 0.1 && pan[1].abs() < 0.1 {
                zoom = 1.0;
                pan = [0.0; 2];
                self.box_resetting = false;
            }
        }
        let delta = self.box_buttons[0] as i32 - self.box_buttons[1] as i32;
        let scale = if inspecting {
            self.inspection_scale()
        } else {
            slot_ui::BOX_VIEW_SCALE
        };
        let max_zoom = self
            .selected_box()
            .map_or(1.0, |(_, _, (w, _, _))| (canvas.w / (w * scale)).max(1.0));
        if self.box_back && inspecting {
            zoom = (zoom * (delta as f32 * dt * 1.4).exp()).clamp(1.0, max_zoom);
        } else if inspecting {
            zoom = 1.0;
            pan = [0.0; 2];
        }
        let dx = self.box_buttons[2] as i32 - self.box_buttons[3] as i32;
        let dy = self.box_buttons[4] as i32 - self.box_buttons[5] as i32;
        pan[0] += dx as f32 * dt * 240.0;
        pan[1] += dy as f32 * dt * 240.0;
        let canvas = self.canvas();
        if let Some((_, _, (w, h, _))) = self.selected_box() {
            let max_x = ((w * scale * zoom - canvas.w) / 2.0).max(0.0);
            let max_y = ((h * scale * zoom - (canvas.h - MOUTH_H)) / 2.0).max(0.0);
            let min_y = -max_y;
            pan[0] = pan[0].clamp(-max_x, max_x);
            pan[1] = pan[1].clamp(min_y, max_y);
        }
        if inspecting {
            self.box_scene = BoxScene::Back { zoom, pan };
        } else {
            self.box_zoom = zoom;
            self.box_pan = pan;
        }
    }

    pub(super) fn return_from_box_inspection(&mut self, zoom: f32, pan: [f32; 2]) {
        self.box_buttons = [false; 6];
        self.box_scene = BoxScene::FlippingFront {
            started: self.now(),
            from: boxview::BoxPose {
                center: [
                    self.inspection_center()[0] + pan[0],
                    self.inspection_center()[1] + pan[1],
                ],
                scale: self.inspection_scale() * zoom,
                angle: self.box_turn,
            },
        };
    }

    pub(super) fn close_open_box(&mut self) {
        self.box_buttons = [false; 6];
        self.box_pending = None;
        self.play_held = None;
        self.box_back = false;
        self.box_scene = if self.box_turn.abs() > 0.001 {
            BoxScene::PreparingClose {
                started: self.now(),
                angle: self.box_turn,
            }
        } else {
            BoxScene::Closing {
                started: self.now(),
            }
        };
    }

    pub(super) fn box_input(&mut self, action: Action) -> bool {
        if !self.on_shelf()
            || self.carousel_view != CarouselView::Boxes
            || self.core_picker.is_some()
            || self.edit.is_some()
        {
            return false;
        }
        if self.box_scene != BoxScene::Carousel && self.selected_box().is_none() {
            self.box_scene = BoxScene::Carousel;
            self.manual_reader = None;
            return false;
        }
        if self.manual_reader.is_some() {
            return self.manual_input(action);
        }
        if self.box_adapter_open {
            if self.box_adapter_closing || self.box_adapter_pickup < 1.0 {
                return true;
            }
            match action {
                Action::GbaDown(Btn::B) => self.box_adapter_closing = true,
                Action::GbaDown(Btn::Left | Btn::Right)
                    if self.box_adapter_core_gpsp && !self.kid_locked() =>
                {
                    self.box_adapter_choice = !self.box_adapter_choice;
                }
                Action::GbaDown(Btn::A) if self.box_adapter_core_gpsp && !self.kid_locked() => {
                    if let (Some(root), Some((cart, _, _))) = (&self.root, self.selected_box()) {
                        let written = slot_store::ini::write(
                            root,
                            AUTO_LINK_FILE,
                            &cart.stem,
                            if self.box_adapter_choice { "yes" } else { "no" },
                        );
                        if let Err(e) = &written {
                            eprintln!("slot: {AUTO_LINK_FILE}: {e}");
                        }
                        self.card_writes.wrote("auto_link_menu.txt", &written);
                    }
                }
                _ => {}
            }
            return true;
        }
        if self.box_art_loading()
            && matches!(action, Action::GbaDown(Btn::B))
            && matches!(
                self.box_scene,
                BoxScene::FlippingBack { .. }
                    | BoxScene::Opening { .. }
                    | BoxScene::QuickOpening { .. }
            )
        {
            self.box_scene = if self.box_inspection_from_open
                && matches!(self.box_scene, BoxScene::FlippingBack { .. })
            {
                BoxScene::Open { item: BoxItem::Box }
            } else {
                BoxScene::Carousel
            };
            self.box_detail_loading = false;
            self.box_cart_loading = false;
            self.box_manual_loading = false;
            return true;
        }
        if matches!(
            self.box_scene,
            BoxScene::FlippingBack { .. }
                | BoxScene::FlippingFront { .. }
                | BoxScene::Opening { .. }
                | BoxScene::QuickOpening { .. }
                | BoxScene::QuickLaunched
                | BoxScene::QuickClosing { .. }
                | BoxScene::PreparingClose { .. }
                | BoxScene::Closing { .. }
        ) {
            self.held_dir = None;
            return true;
        }
        match self.box_scene {
            BoxScene::Carousel => {
                if self.selected_box().is_none() {
                    if !self.shelf().box_checked(self.shelf().index)
                        && self
                            .shelf()
                            .carts
                            .get(self.shelf().index)
                            .is_some_and(|cart| {
                                crate::box_art_fetch::system_id(cart.platform).is_some()
                            })
                        && matches!(
                            action,
                            Action::GbaDown(Btn::A | Btn::Start)
                                | Action::GbaUp(Btn::A)
                                | Action::SelectTap
                                | Action::Insert
                        )
                    {
                        return true;
                    }
                    return false;
                }
                match action {
                    Action::SelectTap => {
                        self.start_box_inspection(false);
                        true
                    }
                    Action::GbaDown(Btn::Select) | Action::GbaUp(Btn::Select) => true,
                    Action::GbaDown(Btn::A) => {
                        self.play_held = Some(self.now());
                        true
                    }
                    Action::GbaUp(Btn::A) => {
                        if self.play_held.take().is_some() {
                            self.start_box_open(false, false);
                        }
                        true
                    }
                    Action::GbaDown(Btn::Start) => {
                        self.start_box_open(true, !self.state.resume_insert);
                        true
                    }
                    Action::Insert => {
                        self.start_box_open(false, false);
                        true
                    }
                    _ => false,
                }
            }
            BoxScene::Back { zoom, pan } => match action {
                _ if self.box_resetting && !matches!(action, Action::GbaUp(_)) => true,
                Action::Favorite | Action::ForgetRecent => true,
                Action::ShelfLeft
                | Action::ShelfRight
                | Action::GbaDown(Btn::Left | Btn::Right)
                    if zoom <= 1.001 =>
                {
                    self.return_from_box_inspection(zoom, pan);
                    true
                }
                Action::PreviousShelf
                | Action::NextShelf
                | Action::ShelfLeft
                | Action::ShelfRight => true,
                Action::GbaDown(Btn::R1) => {
                    self.box_buttons[0] = true;
                    true
                }
                Action::GbaDown(Btn::L1) => {
                    self.box_buttons[1] = true;
                    true
                }
                Action::GbaUp(Btn::R1) => {
                    self.box_buttons[0] = false;
                    true
                }
                Action::GbaUp(Btn::L1) => {
                    self.box_buttons[1] = false;
                    true
                }
                Action::GbaDown(b @ (Btn::Left | Btn::Right | Btn::Up | Btn::Down)) => {
                    if zoom > 1.001 {
                        let at = match b {
                            Btn::Left => 2,
                            Btn::Right => 3,
                            Btn::Up => 4,
                            _ => 5,
                        };
                        self.box_buttons[at] = true;
                    }
                    true
                }
                Action::GbaUp(b @ (Btn::Left | Btn::Right | Btn::Up | Btn::Down)) => {
                    let at = match b {
                        Btn::Left => 2,
                        Btn::Right => 3,
                        Btn::Up => 4,
                        _ => 5,
                    };
                    self.box_buttons[at] = false;
                    true
                }
                Action::GbaDown(Btn::A) | Action::Insert | Action::SelectTap => {
                    if self.box_back
                        || self
                            .selected_box()
                            .is_some_and(|(_, f, _)| f.back_size != (0, 1))
                    {
                        self.box_back = !self.box_back;
                        if !self.box_back {
                            self.box_scene = BoxScene::Back {
                                zoom: 1.0,
                                pan: [0.0; 2],
                            };
                            self.box_buttons = [false; 6];
                            self.box_resetting = false;
                        }
                    }
                    true
                }
                Action::GbaDown(Btn::B) if zoom > 1.001 => {
                    self.box_resetting = true;
                    self.box_buttons = [false; 6];
                    true
                }
                Action::GbaDown(Btn::B) => {
                    self.return_from_box_inspection(zoom, pan);
                    true
                }
                _ => true,
            },
            BoxScene::Open { item } => match action {
                Action::Favorite
                | Action::FavoriteHold
                | Action::ForgetRecent
                | Action::ForgetRecentHold
                | Action::SelectHold
                | Action::PreviousCollection
                | Action::NextCollection
                | Action::PreviousShelf
                | Action::NextShelf => true,
                Action::ShelfLeft | Action::ShelfRight
                    if item == BoxItem::Box && self.box_zoom > 1.001 =>
                {
                    true
                }
                Action::GbaDown(Btn::R1) if item == BoxItem::Box => {
                    self.box_buttons[0] = true;
                    true
                }
                Action::GbaDown(Btn::L1) if item == BoxItem::Box => {
                    self.box_buttons[1] = true;
                    true
                }
                Action::GbaUp(Btn::R1) => {
                    self.box_buttons[0] = false;
                    true
                }
                Action::GbaUp(Btn::L1) => {
                    self.box_buttons[1] = false;
                    true
                }
                Action::GbaDown(b @ (Btn::Left | Btn::Right | Btn::Up | Btn::Down))
                    if item == BoxItem::Box && self.box_zoom > 1.001 =>
                {
                    let at = match b {
                        Btn::Left => 2,
                        Btn::Right => 3,
                        Btn::Up => 4,
                        _ => 5,
                    };
                    self.box_buttons[at] = true;
                    true
                }
                Action::GbaUp(b @ (Btn::Left | Btn::Right | Btn::Up | Btn::Down)) => {
                    let at = match b {
                        Btn::Left => 2,
                        Btn::Right => 3,
                        Btn::Up => 4,
                        _ => 5,
                    };
                    self.box_buttons[at] = false;
                    true
                }
                Action::GbaDown(Btn::B) if item == BoxItem::Box && self.box_zoom > 1.001 => {
                    self.box_resetting = true;
                    self.box_buttons = [false; 6];
                    true
                }
                Action::GbaDown(Btn::B) => {
                    self.close_open_box();
                    true
                }
                Action::GbaDown(Btn::Left) | Action::ShelfLeft => {
                    self.close_open_box();
                    true
                }
                Action::GbaDown(Btn::Right) | Action::ShelfRight => {
                    self.close_open_box();
                    true
                }
                Action::GbaDown(Btn::Down) if item == BoxItem::Cartridge => {
                    self.box_scene = BoxScene::Open {
                        item: if self.box_adapter_available() {
                            BoxItem::Adapter
                        } else {
                            BoxItem::Box
                        },
                    };
                    self.box_pending = None;
                    self.play_held = None;
                    true
                }
                Action::GbaDown(Btn::Up)
                    if item == BoxItem::Cartridge && self.selected_manual_cover().is_some() =>
                {
                    self.box_scene = BoxScene::Open {
                        item: BoxItem::Manual,
                    };
                    self.box_pending = None;
                    self.play_held = None;
                    true
                }
                Action::GbaDown(Btn::Down) if item == BoxItem::Manual => {
                    self.box_scene = BoxScene::Open {
                        item: BoxItem::Cartridge,
                    };
                    true
                }
                Action::GbaDown(Btn::Up) if item == BoxItem::Adapter => {
                    self.box_scene = BoxScene::Open {
                        item: BoxItem::Cartridge,
                    };
                    true
                }
                Action::GbaDown(Btn::Down) if item == BoxItem::Adapter => {
                    self.box_scene = BoxScene::Open { item: BoxItem::Box };
                    true
                }
                Action::GbaDown(Btn::Up) if item == BoxItem::Box => {
                    self.box_scene = BoxScene::Open {
                        item: if self.box_adapter_available() {
                            BoxItem::Adapter
                        } else {
                            BoxItem::Cartridge
                        },
                    };
                    self.box_zoom = 1.0;
                    self.box_pan = [0.0; 2];
                    self.box_resetting = false;
                    self.box_buttons = [false; 6];
                    true
                }
                Action::GbaDown(Btn::Up | Btn::Down) => true,
                Action::GbaUp(Btn::A)
                    if item == BoxItem::Cartridge && self.box_focus.abs() > 0.001 =>
                {
                    if self.play_held.take().is_some() {
                        self.box_pending = Some(BoxCartAction::Play(!self.state.resume_insert));
                    }
                    true
                }
                Action::Insert if item == BoxItem::Cartridge && self.box_focus.abs() > 0.001 => {
                    self.box_pending = Some(BoxCartAction::Play(!self.state.resume_insert));
                    true
                }
                Action::GbaDown(Btn::Start)
                    if item == BoxItem::Cartridge && self.box_focus.abs() > 0.001 =>
                {
                    self.box_pending = Some(BoxCartAction::Picker);
                    true
                }
                Action::SelectTap if item == BoxItem::Cartridge && self.box_focus.abs() > 0.001 => {
                    self.box_pending = Some(BoxCartAction::Studio);
                    true
                }
                Action::GbaDown(Btn::A) | Action::Insert | Action::SelectTap
                    if item == BoxItem::Box =>
                {
                    self.start_box_inspection(true);
                    true
                }
                Action::GbaDown(Btn::Start) if item == BoxItem::Box => true,
                Action::GbaDown(Btn::A) | Action::Insert if item == BoxItem::Manual => {
                    if self.manual_page_count > 0 {
                        self.manual_reader =
                            Some(boxview::ManualReader::new(self.manual_page_count));
                        self.play_held = None;
                    }
                    true
                }
                Action::GbaDown(Btn::A) | Action::Insert if item == BoxItem::Adapter => {
                    if self.box_adapter_available()
                        && (self.box_focus - ADAPTER_FOCUS).abs() < 0.001
                    {
                        self.box_adapter_core_gpsp = self.box_adapter_gpsp();
                        self.box_adapter_choice =
                            self.root.as_ref().zip(self.selected_box()).is_some_and(
                                |(root, (cart, _, _))| auto_link_enabled(root, &cart.stem),
                            );
                        self.box_adapter_open = true;
                        self.box_adapter_closing = false;
                    }
                    true
                }
                Action::GbaDown(Btn::Start | Btn::Select)
                | Action::GbaUp(Btn::A | Btn::Select)
                | Action::SelectTap
                    if matches!(item, BoxItem::Manual | BoxItem::Adapter) =>
                {
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn box_adapter_available(&self) -> bool {
        self.box_adapter_present && self.selected_manual_cover().is_some()
    }

    fn box_adapter_gpsp(&self) -> bool {
        self.root
            .as_ref()
            .zip(self.selected_box())
            .is_some_and(|(root, (cart, _, _))| {
                slot_store::core_for_platform(root, &cart.stem, cart.platform, self.state.gb_core)
                    == Core::Gpsp
            })
    }

    pub(super) fn wants_auto_link(&self, cart: &Cart) -> bool {
        !self.kid_locked()
            && adapter_game(cart)
            && self.root.as_ref().is_some_and(|root| {
                auto_link_enabled(root, &cart.stem)
                    && slot_store::core_for_platform(
                        root,
                        &cart.stem,
                        cart.platform,
                        self.state.gb_core,
                    ) == Core::Gpsp
            })
    }

    pub fn set_box_adapter_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.box_adapter_faces = faces;
    }

    pub(super) fn manual_input(&mut self, action: Action) -> bool {
        let canvas = self.canvas();
        let now = self.now();
        let reader = self.manual_reader.as_mut().expect("manual reader");
        match action {
            Action::GbaUp(Btn::R1) => reader.buttons[0] = false,
            Action::GbaUp(Btn::L1) => reader.buttons[1] = false,
            Action::GbaUp(b @ (Btn::Left | Btn::Right | Btn::Up | Btn::Down)) => {
                let at = match b {
                    Btn::Left => 2,
                    Btn::Right => 3,
                    Btn::Up => 4,
                    _ => 5,
                };
                reader.buttons[at] = false;
            }
            _ if reader.closing || reader.pickup < 1.0 => {}
            Action::GbaDown(Btn::R1) if reader.turn.is_none() => reader.buttons[0] = true,
            Action::GbaDown(Btn::L1) if reader.turn.is_none() => reader.buttons[1] = true,
            Action::GbaDown(Btn::B) if reader.zoom > 1.001 || reader.resetting => {
                reader.resetting = true;
                reader.buttons = [false; 6];
            }
            Action::GbaDown(Btn::B) if reader.turn.is_none() => {
                if reader.page == 0 {
                    reader.closing = true;
                    reader.buttons = [false; 6];
                } else {
                    manual_turn(reader, canvas, 0, true);
                }
            }
            Action::GbaDown(b @ (Btn::Left | Btn::Right))
                if reader.turn.is_none() && !reader.resetting =>
            {
                let right = b == Btn::Right;
                let at = if right { 3 } else { 2 };
                if reader.buttons[at] {
                    return true;
                }
                reader.buttons[at] = true;
                if reader.zoom <= 1.001 && !reader.buttons[0] {
                    let (max_x, _) = manual_bounds(reader, canvas);
                    let edge = if right {
                        reader.pan[0] <= -max_x + 0.5
                    } else {
                        reader.pan[0] >= max_x - 0.5
                    };
                    let double = reader.last_tap.is_some_and(|(direction, at)| {
                        direction == right
                            && now.saturating_sub(at) <= boxview::MANUAL_DOUBLE_TAP_MS
                    });
                    reader.last_tap = Some((right, now));
                    if edge || double {
                        let spread = reader.spread(reader.page);
                        let next = if right {
                            (spread[1].unwrap_or(reader.page) + 1).min(reader.count - 1)
                        } else {
                            reader.page.saturating_sub(1)
                        };
                        let next = reader.spread(next)[0].unwrap_or(next);
                        if next != reader.page {
                            manual_turn(reader, canvas, next, false);
                        }
                    }
                }
            }
            Action::GbaDown(b @ (Btn::Up | Btn::Down))
                if reader.zoom > 1.001 && !reader.resetting =>
            {
                reader.buttons[if b == Btn::Up { 4 } else { 5 }] = true;
            }
            _ => {}
        }
        true
    }

    pub fn box_face_wants(&self) -> Vec<FaceWant> {
        if self.carousel_view == CarouselView::Cartridges {
            return Vec::new();
        }
        let here = self.shelf();
        let radius = Shelf::box_visible_radius(self.canvas());
        let screen = here.around(radius);
        let mut wants: Vec<(usize, usize)> = screen.iter().map(|i| (self.shelf_at, *i)).collect();
        for at in self.neighbour_shelves() {
            wants.extend(
                self.shelves[at]
                    .1
                    .around(radius)
                    .into_iter()
                    .map(|i| (at, i)),
            );
        }
        wants.extend(
            here.around(FACE_AHEAD.max(radius + 2))
                .into_iter()
                .filter(|i| !screen.contains(i))
                .map(|i| (self.shelf_at, i)),
        );
        wants
            .into_iter()
            .filter(|(at, i)| {
                !self.shelves[*at].1.box_checked(*i)
                    && crate::box_art_fetch::system_id(self.shelves[*at].1.carts[*i].platform)
                        .is_some()
            })
            .map(|(shelf, index)| FaceWant {
                shelf,
                index,
                on_screen: shelf == self.shelf_at && screen.contains(&index),
            })
            .collect()
    }

    pub fn box_detail_want(&self) -> Option<(usize, usize, bool, bool)> {
        let (_, faces, _) = self.selected_box()?;
        let back = matches!(
            self.box_scene,
            BoxScene::FlippingBack { .. }
                | BoxScene::Back { .. }
                | BoxScene::FlippingFront { .. }
                | BoxScene::Opening { .. }
                | BoxScene::Open { .. }
                | BoxScene::Closing { .. }
                | BoxScene::PreparingClose { .. }
        ) && faces.back == faces.front
            && faces.back_size != (0, 1);
        let flap = matches!(
            self.box_scene,
            BoxScene::Opening { .. }
                | BoxScene::QuickOpening { .. }
                | BoxScene::QuickLaunched
                | BoxScene::QuickClosing { .. }
                | BoxScene::Open { .. }
                | BoxScene::PreparingClose { .. }
                | BoxScene::Closing { .. }
        ) && faces.flap.is_none();
        (back || flap).then_some((self.shelf_at, self.shelf().index, back, flap))
    }

    pub fn box_prefetch_wants(&self) -> Vec<(usize, usize)> {
        if self.carousel_view != CarouselView::Boxes
            || !(self.on_shelf() || matches!(self.box_scene, BoxScene::QuickLaunched))
        {
            return Vec::new();
        }
        self.shelf()
            .around(if self.on_shelf() { 1 } else { 0 })
            .into_iter()
            .filter(|&index| self.shelf().box_faces(index).is_some())
            .map(|index| (self.shelf_at, index))
            .collect()
    }

    pub fn boxes_active(&self) -> bool {
        self.on_shelf() && self.carousel_view == CarouselView::Boxes
    }

    pub fn box_view_enabled(&self) -> bool {
        self.carousel_view == CarouselView::Boxes
    }

    pub fn selected_box_key(&self) -> Option<(usize, usize)> {
        self.selected_box()
            .map(|_| (self.shelf_at, self.shelf().index))
    }

    pub fn selected_box_seat(&self) -> (usize, usize) {
        (self.shelf_at, self.shelf().index)
    }

    pub fn box_resize_carts(&self) -> Vec<Cart> {
        let mut seen = std::collections::HashSet::new();
        self.shelves
            .iter()
            .flat_map(|(_, shelf)| &shelf.carts)
            .filter(|cart| {
                crate::box_art_fetch::system_id(cart.platform).is_some()
                    && seen.insert(cart.rom.clone())
            })
            .cloned()
            .collect()
    }

    pub fn set_box_detail_loading(&mut self, loading: bool) {
        self.box_detail_loading = loading;
    }

    pub fn box_cart_is_loading(&self) -> bool {
        matches!(
            self.box_scene,
            BoxScene::Opening { .. } | BoxScene::QuickOpening { .. }
        ) && self.shelf().face(self.shelf().index).is_none()
    }

    pub fn set_box_cart_loading(&mut self, loading: bool) {
        self.box_cart_loading = loading;
    }

    pub fn set_box_manual_loading(&mut self, loading: bool) {
        self.box_manual_loading = loading;
    }

    pub fn take_prefetched_cart_faces(&mut self, shelf: usize, index: usize) -> Vec<TexId> {
        if !self.boxes_active() || self.bundle_seats().contains(&(shelf, index)) {
            return Vec::new();
        }
        let Some((_, shelf)) = self.shelves.get_mut(shelf) else {
            return Vec::new();
        };
        let mut textures = Vec::new();
        textures.extend(shelf.take_face(index));
        textures.extend(shelf.take_interior(index));
        textures
    }

    pub fn box_faces_at(&self, shelf: usize, index: usize) -> Option<slot_ui::BoxFaceTextures> {
        self.shelves.get(shelf)?.1.box_faces(index)
    }

    pub fn box_back_unavailable(&mut self, shelf: usize, index: usize) {
        if shelf == self.shelf_at && index == self.shelf().index {
            self.box_back = false;
        }
    }

    pub fn shed_box_faces(&mut self) -> Vec<TexId> {
        let neighbours = self.neighbour_shelves();
        let radius = Shelf::box_visible_radius(self.canvas());
        let mut shed = Vec::new();
        for (at, (_, shelf)) in self.shelves.iter_mut().enumerate() {
            let keep = if self.carousel_view == CarouselView::Cartridges {
                Vec::new()
            } else if at == self.shelf_at {
                shelf.around(FACE_KEEP.max(radius + 2))
            } else if neighbours.contains(&at) {
                shelf.around(radius)
            } else {
                Vec::new()
            };
            for i in 0..shelf.carts.len() {
                if !keep.contains(&i) {
                    if let Some(faces) = shelf.take_box_faces(i) {
                        shed.extend([faces.front, faces.side]);
                        if faces.back != faces.front {
                            shed.push(faces.back);
                        }
                        shed.extend(faces.flap);
                    }
                }
            }
        }
        shed
    }

    pub(super) fn draw_box_scene(&self, out: &mut Vec<Draw>) -> bool {
        if self.carousel_view != CarouselView::Boxes || self.box_scene == BoxScene::Carousel {
            return false;
        }
        let Some((_, faces, (front_w, front_h, depth))) = self.selected_box() else {
            return false;
        };
        let canvas = self.canvas();
        let now = self.now();
        match self.box_scene {
            BoxScene::FlippingBack { started, from }
            | BoxScene::FlippingFront { started, from } => {
                let to_back = matches!(self.box_scene, BoxScene::FlippingBack { .. });
                let t = (now.saturating_sub(started) as f32 / BOX_FLIP_MS as f32).clamp(0.0, 1.0);
                let eased = t * t * (3.0 - 2.0 * t);
                let (center, scale, angle) = if to_back {
                    (self.inspection_center(), self.inspection_scale(), 0.0)
                } else if self.box_inspection_from_open {
                    (
                        [canvas.w / 2.0, canvas.h / 2.0],
                        slot_ui::BOX_VIEW_SCALE,
                        0.0,
                    )
                } else {
                    self.shelf().selected_box_pose(canvas)
                };
                let center = [
                    from.center[0] + (center[0] - from.center[0]) * eased,
                    from.center[1] + (center[1] - from.center[1]) * eased,
                ];
                let scale = from.scale + (scale - from.scale) * eased;
                let angle = from.angle + (angle - from.angle) * eased;
                slot_ui::draw_rotated_box(
                    faces,
                    (front_w * scale, front_h * scale, depth * scale),
                    angle,
                    center,
                    1.0,
                    out,
                );
            }
            BoxScene::Back { zoom, pan } => {
                let scale = self.inspection_scale() * zoom;
                let center = self.inspection_center();
                slot_ui::draw_rotated_box(
                    faces,
                    (front_w * scale, front_h * scale, depth * scale),
                    self.box_turn,
                    [center[0] + pan[0], center[1] + pan[1]],
                    1.0,
                    out,
                );
            }
            BoxScene::Opening { started } => {
                let t = (now.saturating_sub(started + BOX_BAR_FADE_MS) as f32 / BOX_OPEN_MS as f32)
                    .clamp(0.0, 1.0);
                self.draw_open_box(t, out);
            }
            BoxScene::QuickOpening { started } => {
                let t = ((self.clock.round() as Millis).saturating_sub(started) as f32
                    / BOX_QUICK_MS as f32)
                    .clamp(0.0, 1.0);
                self.draw_quick_box(t, out);
            }
            BoxScene::Closing { started } => {
                let t = (now.saturating_sub(started) as f32 / BOX_CLOSE_MS as f32).clamp(0.0, 1.0);
                self.draw_open_box(1.0 - t, out);
            }
            BoxScene::QuickClosing { started } => {
                let t = ((self.clock.round() as Millis).saturating_sub(started) as f32
                    / BOX_QUICK_MS as f32)
                    .clamp(0.0, 1.0);
                self.draw_quick_box(1.0 - t, out);
            }
            BoxScene::Open { .. } | BoxScene::PreparingClose { .. } => self.draw_open_box(1.0, out),
            BoxScene::QuickLaunched => self.draw_open_box(1.0, out),
            BoxScene::Carousel => return false,
        }
        if self.box_art_loading()
            && matches!(
                self.box_scene,
                BoxScene::FlippingBack { .. }
                    | BoxScene::Opening { .. }
                    | BoxScene::QuickOpening { .. }
            )
        {
            let phase = (now / 120 % 4) as usize;
            for i in 0..4 {
                out.push(Draw::Rect {
                    x: canvas.w / 2.0 - 20.0 + i as f32 * 12.0,
                    y: canvas.h - 36.0,
                    w: 7.0,
                    h: 7.0,
                    colour: [1.0, 1.0, 1.0, if i == phase { 1.0 } else { 0.3 }],
                });
            }
        }
        true
    }

    fn draw_quick_box(&self, progress: f32, out: &mut Vec<Draw>) {
        let Some((cart, faces, (front_w, front_h, depth))) = self.selected_box() else {
            return;
        };
        let canvas = self.canvas();
        let t = progress.clamp(0.0, 1.0);
        let smooth = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let move_box = slot_ui::ease(t);
        let origin = if matches!(self.box_scene, BoxScene::QuickClosing { .. }) {
            let (center, scale, angle) = self.shelf().selected_box_pose(canvas);
            boxview::BoxPose {
                center,
                scale,
                angle,
            }
        } else {
            self.box_open_from
        };
        let scale = origin.scale + (slot_ui::BOX_VIEW_SCALE - origin.scale) * move_box;
        let (w, h) = (front_w * scale, front_h * scale);
        let center = [
            origin.center[0] + (canvas.w / 2.0 - origin.center[0]) * move_box,
            origin.center[1]
                + (canvas.h
                    + front_h * slot_ui::BOX_VIEW_SCALE / 2.0
                    + front_w * slot_ui::BOX_VIEW_SCALE * (176.0 / 700.0)
                    - origin.center[1])
                    * move_box,
        ];
        let lid = (std::f32::consts::FRAC_PI_2 * smooth(t * 2.0)).sin();
        let tab = smooth((t - 0.5) * 2.0);
        draw_flap(faces.flap, center, w, h, lid, tab, out);
        let target = slot_ui::aligned_rect(canvas, cart.platform);
        let (small_w, small_h) = slot_ui::loose_cart_dimensions(cart);
        let small_w = small_w * origin.scale;
        let small_h = small_h * origin.scale;
        let start = [
            origin.center[0] - small_w / 2.0,
            origin.center[1] - small_h / 2.0,
            small_w,
            small_h,
        ];
        let rect = std::array::from_fn(|i| start[i] + (target[i] - start[i]) * move_box);
        self.shelf().draw_cart_at(self.shelf().index, rect, out);
        slot_ui::draw_rotated_box(
            faces,
            (w, h, depth * scale),
            origin.angle * (1.0 - move_box),
            center,
            1.0,
            out,
        );
    }

    fn open_box_positions(
        &self,
        cart: &Cart,
        box_height: f32,
        progress: f32,
        show_contents: bool,
    ) -> (f32, f32) {
        let canvas = self.canvas();
        let smooth = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let ms = progress * BOX_OPEN_MS as f32;
        let extraction = smooth((ms - 710.0) / 420.0);
        let move_to_center = smooth((ms - 1330.0) / 170.0)
            * (1.0
                - smooth(
                    (self.box_focus - BOX_FOCUS_CART_DOWN)
                        / (BOX_FOCUS_BOX_MOVE - BOX_FOCUS_CART_DOWN),
                ));
        let pickup = smooth((ms - 1500.0) / 150.0)
            * (1.0 - smooth(self.box_focus.abs() / BOX_FOCUS_CART_DOWN));
        let (_, native_h) = slot_ui::cart_box(cart.platform);
        let native_h = native_h as f32;
        let target_y = slot_ui::rest_y(canvas, native_h) + native_h / 2.0;
        let manual_h = slot_ui::manual_dimensions().1 * slot_ui::BOX_VIEW_SCALE;
        let small_h = slot_ui::loose_cart_dimensions(cart).1 * slot_ui::BOX_VIEW_SCALE;
        let adapter_h = small_h * 37.7 / 35.0;
        let has_adapter = show_contents && self.box_adapter_available();
        let adapter_space = if has_adapter {
            adapter_h + STACK_GAP
        } else {
            0.0
        };
        let manual_shift =
            (STACK_GAP + (manual_h + small_h) / 2.0) * smooth(-self.box_focus) * progress;
        let parked_y = canvas.h + box_height * 0.15;
        let mut box_y = parked_y
            + (canvas.h / 2.0 - parked_y)
                * smooth((self.box_focus - BOX_FOCUS_BOX_MOVE) / (1.0 - BOX_FOCUS_BOX_MOVE))
            + manual_shift;
        let small_center_y = box_y - box_height / 2.0 - STACK_GAP - adapter_space - small_h / 2.0;
        let emerging_y = box_y - adapter_space / 2.0;
        let cy = emerging_y + (small_center_y - emerging_y) * extraction;
        let cart_height = small_h + (native_h - small_h) * pickup;
        let mut cart_y = cy + (target_y + manual_shift - cy) * move_to_center;
        if has_adapter {
            let adapter_center = cart_y + cart_height / 2.0 + STACK_GAP + adapter_h / 2.0;
            let adapter_focus = smooth(self.box_adapter_focus) * progress;
            let shift = (target_y - adapter_center) * adapter_focus;
            box_y += shift;
            cart_y += shift;
            if extraction >= 1.0 {
                let adapter_bottom = cart_y + cart_height / 2.0 + STACK_GAP + adapter_h;
                box_y += (adapter_bottom + STACK_GAP - (box_y - box_height / 2.0)).max(0.0);
            }
        }
        (box_y, cart_y)
    }

    pub(super) fn draw_open_box(&self, progress: f32, out: &mut Vec<Draw>) {
        let Some((cart, faces, (front_w, front_h, depth))) = self.selected_box() else {
            return;
        };
        let canvas = self.canvas();
        let p = progress.clamp(0.0, 1.0);
        let smooth = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let ms = p * BOX_OPEN_MS as f32;
        let travel = smooth(ms / 280.0);
        let extraction = smooth((ms - 710.0) / 420.0);
        let pickup = smooth((ms - 1500.0) / 150.0)
            * (1.0 - smooth(self.box_focus.abs() / BOX_FOCUS_CART_DOWN));
        let flap = smooth((ms - 280.0) / 430.0) * (1.0 - smooth((ms - 1130.0) / 370.0));
        let lid = (std::f32::consts::FRAC_PI_2 * smooth(flap * 2.0)).sin();
        let tab = smooth((flap - 0.5) * 2.0);
        let scale = slot_ui::BOX_VIEW_SCALE;
        let (w, h) = (front_w * scale, front_h * scale);
        let (_, ch) = slot_ui::cart_box(cart.platform);
        let target_y = slot_ui::rest_y(canvas, ch as f32) + ch as f32 / 2.0;
        let (manual_w, manual_h) = slot_ui::manual_dimensions();
        let (manual_w, manual_h) = (manual_w * scale, manual_h * scale);
        let (small_w, small_h) = slot_ui::loose_cart_dimensions(cart);
        let (small_w, small_h) = (small_w * scale, small_h * scale);
        let show_contents = !matches!(
            self.box_scene,
            BoxScene::QuickOpening { .. } | BoxScene::QuickLaunched | BoxScene::QuickClosing { .. }
        );
        let has_adapter = show_contents && self.box_adapter_available();
        let adapter_w = small_w * 62.0 / 60.0;
        let adapter_h = small_h * 37.7 / 35.0;
        let (box_y, cart_center_y) = self.open_box_positions(cart, h, p, show_contents);
        let origin = if matches!(
            self.box_scene,
            BoxScene::Closing { .. } | BoxScene::QuickClosing { .. }
        ) {
            let (center, scale, angle) = self.shelf().selected_box_pose(canvas);
            boxview::BoxPose {
                center,
                scale,
                angle,
            }
        } else {
            self.box_open_from
        };
        let (cw, ch) = slot_ui::cart_box(cart.platform);
        let (cw, ch) = (cw as f32, ch as f32);
        let cart_width = small_w + (cw - small_w) * pickup;
        let cart_height = small_h + (ch - small_h) * pickup;
        let zoom = 1.0 + (self.box_zoom - 1.0) * self.box_focus * travel;
        let center = [
            origin.center[0] + (canvas.w / 2.0 + self.box_pan[0] - origin.center[0]) * travel,
            origin.center[1] + (box_y + self.box_pan[1] - origin.center[1]) * travel,
        ];
        let face_scale = (origin.scale + (scale - origin.scale) * travel) * zoom;
        let angle = origin.angle * (1.0 - travel) + self.box_turn * travel;
        draw_flap(faces.flap, center, w, h, lid, tab, out);
        let draw_box = |out: &mut Vec<Draw>| {
            slot_ui::draw_rotated_box(
                faces,
                (
                    front_w * face_scale,
                    front_h * face_scale,
                    depth * face_scale,
                ),
                angle,
                center,
                1.0,
                out,
            );
        };
        let box_behind_cart = match self.box_scene {
            BoxScene::Opening { .. }
            | BoxScene::QuickOpening { .. }
            | BoxScene::QuickClosing { .. } => ms >= 1500.0,
            BoxScene::Open { .. } => self.box_focus <= BOX_FOCUS_BOX_MOVE,
            _ => false,
        };
        if box_behind_cart {
            draw_box(out);
        }
        if extraction > 0.0 && show_contents {
            if let Some(tex) = self
                .selected_manual_cover()
                .filter(|_| self.manual_reader.is_none() || self.box_focus > -0.999)
            {
                let behind = cart_center_y - manual_h / 2.0;
                let target = cart_center_y - cart_height / 2.0 - STACK_GAP - manual_h;
                let slide_out = smooth((ms - 1130.0) / 370.0);
                out.push(Draw::Tex {
                    x: canvas.w / 2.0 - manual_w / 2.0,
                    y: behind + (target - behind) * slide_out,
                    w: manual_w,
                    h: manual_h,
                    tex,
                    alpha: 1.0,
                });
            }
        }
        if extraction > 0.0 {
            self.shelf().draw_cart_at(
                self.shelf().index,
                [
                    canvas.w / 2.0 - cart_width / 2.0,
                    cart_center_y - cart_height / 2.0,
                    cart_width,
                    cart_height,
                ],
                out,
            );
        }
        if extraction > 0.0 && has_adapter {
            if let Some(sprites) = self.link_sprites {
                let y = cart_center_y + cart_height / 2.0 + STACK_GAP;
                let pickup = smooth(self.box_adapter_pickup);
                let full_w = sprites.adapter.w as f32;
                let full_h = sprites.adapter.h as f32;
                let x = (canvas.w - adapter_w) / 2.0;
                let full_x = (canvas.w - full_w) / 2.0;
                let full_y = ((canvas.h - MOUTH_H - full_h) / 2.0 - 35.0).max(0.0);
                if pickup > 0.0 {
                    out.push(Draw::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: canvas.w,
                        h: canvas.h - MOUTH_H,
                        colour: [0.0, 0.0, 0.0, 0.5 * pickup],
                    });
                }
                out.push(Draw::Tex {
                    x: x + (full_x - x) * pickup,
                    y: y + (full_y - y) * pickup,
                    w: adapter_w + (full_w - adapter_w) * pickup,
                    h: adapter_h + (full_h - adapter_h) * pickup,
                    tex: sprites.adapter.tex,
                    alpha: 1.0,
                });
                if self.box_adapter_open && pickup > 0.0 {
                    let face = if self.kid_locked() {
                        3
                    } else if self.box_adapter_core_gpsp {
                        if self.box_adapter_choice {
                            1
                        } else {
                            2
                        }
                    } else {
                        0
                    };
                    if let Some(&(tex, width, height)) = self.box_adapter_faces.get(face) {
                        let width = (width as f32).min(canvas.w - 20.0);
                        out.push(Draw::Tex {
                            x: (canvas.w - width) / 2.0,
                            y: full_y + full_h + 8.0,
                            w: width,
                            h: height as f32,
                            tex,
                            alpha: pickup,
                        });
                    }
                }
            }
        }
        if !box_behind_cart {
            draw_box(out);
        }
        if self.manual_reader.is_some() && self.box_focus <= -0.999 {
            self.draw_manual_reader(target_y, out);
        }
    }

    pub(super) fn draw_manual_reader(&self, target_y: f32, out: &mut Vec<Draw>) {
        let Some(reader) = &self.manual_reader else {
            return;
        };
        let canvas = self.canvas();
        let progress = reader.pickup * reader.pickup * (3.0 - 2.0 * reader.pickup);
        let (small_w, small_h) = slot_ui::manual_dimensions();
        let small_w = small_w * slot_ui::BOX_VIEW_SCALE;
        let small_h = small_h * slot_ui::BOX_VIEW_SCALE;
        let (width, height) = manual_page_size(canvas);
        let zoom = 1.0 + (reader.zoom - 1.0) * progress;
        let center_y =
            target_y + ((canvas.h - MOUTH_H) / 2.0 - target_y + reader.pan[1]) * progress;
        let w = small_w + (width * zoom - small_w) * progress;
        let h = small_h + (height * zoom - small_h) * progress;
        let top = center_y - h / 2.0;
        let sides = |page| {
            let [left, right] = reader.spread(page);
            if right.is_some() {
                [left, right]
            } else if page == 0 {
                [None, left]
            } else {
                [left, None]
            }
        };
        let draw_page = |out: &mut Vec<Draw>, index: Option<usize>, x: f32| {
            if let Some(index) = index {
                if let Some(tex) = self.manual_page_texture(index) {
                    out.push(Draw::Tex {
                        x,
                        y: top,
                        w,
                        h,
                        tex,
                        alpha: 1.0,
                    });
                } else {
                    out.push(Draw::Rect {
                        x,
                        y: top,
                        w,
                        h,
                        colour: [0.85, 0.85, 0.85, 1.0],
                    });
                }
            }
        };
        if let Some(turn) = &reader.turn {
            let t = (turn.elapsed / boxview::MANUAL_TURN_SECONDS).clamp(0.0, 1.0);
            let smooth = t * t * (3.0 - 2.0 * t);
            let spine = turn.from_spine + (turn.to_spine - turn.from_spine) * smooth;
            let forward = turn.to > turn.from;
            let old = sides(turn.from);
            let new = sides(turn.to);
            let (left, right, source, reverse) = if forward {
                (old[0], new[1], old[1], new[0])
            } else {
                (new[0], old[1], old[0], new[1])
            };
            draw_page(out, left, spine - w);
            draw_page(out, right, spine);
            let page = if t < 0.5 { source } else { reverse };
            if let Some(tex) = page.and_then(|index| self.manual_page_texture(index)) {
                let direction = if forward { 1.0 } else { -1.0 };
                let angle = std::f32::consts::PI * smooth;
                let bend = angle.sin();
                let pos = |u: f32| {
                    let x = spine
                        + direction
                            * w
                            * (u * angle.cos() + 0.18 * bend * (std::f32::consts::PI * u).sin());
                    let rise = h * 0.04 * bend * (std::f32::consts::PI * u).sin();
                    (x, rise)
                };
                for i in 0..16 {
                    let a = i as f32 / 16.0;
                    let b = (i + 1) as f32 / 16.0;
                    let (xa, ra) = pos(a);
                    let (xb, rb) = pos(b);
                    let normal = if t < 0.5 { forward } else { !forward };
                    let (u, du) = if normal { (a, b - a) } else { (1.0 - a, a - b) };
                    out.push(Draw::Quad {
                        corners: [
                            [xa, top - ra],
                            [xb, top - rb],
                            [xb, top + h + rb],
                            [xa, top + h + ra],
                        ],
                        tex,
                        uv: [u, 0.0, du, 1.0],
                        alpha: 1.0,
                    });
                }
            }
        } else {
            let spine = if progress < 1.0 {
                canvas.w / 2.0 - w / 2.0
            } else {
                manual_spine(reader, canvas)
            };
            let [left, right] = sides(reader.page);
            draw_page(out, left, spine - w);
            draw_page(out, right, spine);
        }
    }

    pub(super) fn draw_open_box_backdrop(
        &self,
        recede: f32,
        dim: f32,
        distance: f32,
        out: &mut Vec<Draw>,
    ) {
        let Some((cart, faces, (w, h, depth))) = self.selected_box() else {
            return;
        };
        let canvas = self.canvas();
        let scale = slot_ui::BOX_VIEW_SCALE;
        if matches!(self.box_scene, BoxScene::QuickLaunched) {
            return;
        }
        if !matches!(
            self.box_scene,
            BoxScene::QuickOpening { .. } | BoxScene::QuickLaunched | BoxScene::QuickClosing { .. }
        ) {
            if let Some(tex) = self.selected_manual_cover() {
                let (mw, mh) = slot_ui::manual_dimensions();
                let (mw, mh) = (mw * scale, mh * scale);
                let (_, ch) = slot_ui::cart_box(cart.platform);
                out.push(Draw::Tex {
                    x: canvas.w / 2.0 - mw / 2.0,
                    y: slot_ui::rest_y(canvas, ch as f32) - 12.0 - mh - distance * recede,
                    w: mw,
                    h: mh,
                    tex,
                    alpha: (1.0 - recede) * dim,
                });
            }
        }
        slot_ui::draw_rotated_box(
            faces,
            (w * scale, h * scale, depth * scale),
            self.box_turn,
            [
                canvas.w / 2.0,
                canvas.h + h * scale * 0.15 + distance * recede,
            ],
            (1.0 - recede) * dim,
            out,
        );
    }
}
