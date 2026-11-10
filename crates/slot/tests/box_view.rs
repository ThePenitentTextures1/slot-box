mod common;

use slot::app::{App, Phase};
use slot_input::{Action, Btn};
use slot_store::{write_carousel_view, CarouselView};
use slot_ui::{BoxFaceTextures, Draw, TexId};

fn textures(id: usize) -> BoxFaceTextures {
    BoxFaceTextures {
        front: TexId::from_raw(id),
        side: TexId::from_raw(id + 1),
        back: TexId::from_raw(id + 2),
        flap: Some(TexId::from_raw(id + 3000)),
        front_size: (100, 100),
        side_size: (20, 100),
        back_size: (100, 100),
        japanese: false,
        pivot: [120.0, 50.0],
        front_center: [170.0, 50.0],
    }
}

fn setup(root: &std::path::Path) -> App {
    write_carousel_view(root, CarouselView::Boxes).unwrap();
    let mut app = common::boot(root);
    for want in app.box_face_wants() {
        app.set_box_faces(
            want.shelf,
            want.index,
            Some(textures(1000 + want.index * 3)),
        );
    }
    for want in app.face_wants() {
        app.set_cart_face(want.shelf, want.index, TexId::from_raw(2000 + want.index));
    }
    app
}

fn face(app: &App, tex: TexId) -> Option<[[f32; 2]; 4]> {
    let mut out = Vec::new();
    app.draw(&mut out);
    out.iter()
        .filter_map(|draw| match *draw {
            Draw::PerspectiveQuad {
                tex: t, corners, ..
            } if t == tex => Some(corners),
            Draw::Tex {
                tex: t, x, y, w, h, ..
            } if t == tex => Some([[x, y], [x + w, y], [x + w, y + h], [x, y + h]]),
            _ => None,
        })
        .min_by(|a, b| {
            let distance =
                |q: &[[f32; 2]; 4]| ((q[0][0] + q[1][0]) / 2.0 - app.canvas().w / 2.0).abs();
            distance(a).total_cmp(&distance(b))
        })
}

fn equal_pose(a: [[f32; 2]; 4], b: [[f32; 2]; 4]) {
    for (a, b) in a.iter().zip(b) {
        assert!((a[0] - b[0]).abs() < 0.001, "{a:?} != {b:?}");
        assert!((a[1] - b[1]).abs() < 0.001, "{a:?} != {b:?}");
    }
}

#[test]
fn flipping_uses_the_spine_and_returns_to_the_carousel() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    let tex = textures(1000);
    let before = face(&app, tex.front).unwrap();
    app.apply(Action::GbaDown(Btn::L2));
    equal_pose(before, face(&app, tex.front).unwrap());
    assert!(
        app.shed_box_faces().is_empty(),
        "the flip evicted visible neighbours"
    );

    app.update(0.08);
    assert!(face(&app, tex.front).is_some());
    assert!(face(&app, tex.side).is_some());
    app.update(0.08);
    assert!(face(&app, tex.side).is_some());
    app.update(0.08);
    assert!(face(&app, tex.side).is_some());
    assert!(face(&app, tex.back).is_some());
    app.update(0.09);
    let back = face(&app, tex.back).unwrap();

    app.apply(Action::GbaDown(Btn::B));
    equal_pose(back, face(&app, tex.back).unwrap());
    app.update(0.08);
    assert!(face(&app, tex.back).is_some());
    assert!(face(&app, tex.side).is_some());
    app.update(0.25);
    equal_pose(before, face(&app, tex.front).unwrap());
}

#[test]
fn flipping_while_scrolling_starts_at_the_current_box_pose() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta", "Gamma"]);
    let mut app = setup(root.path());
    app.apply(Action::ShelfRight);
    app.update(0.06);
    let tex = textures(1003);
    let front = face(&app, tex.front).unwrap();
    let side = face(&app, tex.side).unwrap();
    app.apply(Action::GbaDown(Btn::L2));
    equal_pose(front, face(&app, tex.front).unwrap());
    equal_pose(side, face(&app, tex.side).unwrap());
}

#[test]
fn carousel_inspection_returns_on_horizontal_navigation() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    app.apply(Action::SelectTap);
    app.update(0.33);
    assert!(face(&app, textures(1000).back).is_some());
    app.apply(Action::ShelfLeft);
    app.update(0.33);
    assert!(face(&app, textures(1000).front).is_some());
    assert_eq!(app.selected_stem(), Some("Alpha"));
}

#[test]
fn flipping_back_from_zoom_preserves_the_initial_pose() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    let tex = textures(1000);
    app.apply(Action::GbaDown(Btn::L2));
    app.update(0.33);
    app.apply(Action::GbaDown(Btn::R1));
    app.update(0.5);
    app.apply(Action::GbaUp(Btn::R1));
    let zoomed = face(&app, tex.back).unwrap();
    app.apply(Action::GbaDown(Btn::B));
    equal_pose(zoomed, face(&app, tex.back).unwrap());
    app.update(1.0);
    let reset = face(&app, tex.back).unwrap();
    assert!(reset[1][0] - reset[0][0] < zoomed[1][0] - zoomed[0][0]);
    app.apply(Action::GbaDown(Btn::B));
    equal_pose(reset, face(&app, tex.back).unwrap());
}

fn open(app: &mut App) {
    app.apply(Action::GbaDown(Btn::A));
    app.apply(Action::GbaUp(Btn::A));
    app.update(1.21);
}

#[test]
fn cartridge_actions_wait_until_it_is_picked_up() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    open(&mut app);
    app.apply(Action::GbaDown(Btn::Down));
    app.update(1.0);
    app.apply(Action::GbaDown(Btn::Up));
    app.apply(Action::GbaDown(Btn::A));
    app.apply(Action::GbaUp(Btn::A));
    assert!(matches!(app.phase(), Phase::Shelf));
    app.update(1.0);
    assert!(matches!(app.phase(), Phase::Inserting { .. }));
}

#[test]
fn vertical_open_box_preserves_scale_and_cartridge_geometry() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    let box_tex = textures(1000).front;
    let cart_tex = TexId::from_raw(2000);
    let before = face(&app, box_tex).unwrap();
    app.apply(Action::GbaDown(Btn::A));
    app.apply(Action::GbaUp(Btn::A));
    equal_pose(before, face(&app, box_tex).unwrap());
    app.update(0.36);
    let mut scene = Vec::new();
    app.draw(&mut scene);
    assert!(scene.iter().any(|draw| matches!(draw, Draw::Part { tex, .. } if *tex == TexId::from_raw(4000))));
    app.update(0.72);
    let centered_small = face(&app, cart_tex).unwrap();
    let box_face = face(&app, box_tex).unwrap();
    assert!(((centered_small[1][0] - centered_small[0][0]) / (box_face[1][0] - box_face[0][0]) - 60.0 / 125.0).abs() < 0.001);
    let (_, native_h) = slot_ui::cart_box(slot_store::Platform::Gba);
    let cart_center = (centered_small[0][1] + centered_small[3][1]) / 2.0;
    assert!((cart_center - slot_ui::rest_y(native_h as f32) - native_h as f32 / 2.0).abs() < 0.01);
    app.update(0.13);
    let picked = face(&app, cart_tex).unwrap();
    let (w, h) = slot_ui::cart_box(slot_store::Platform::Gba);
    assert!((picked[1][0] - picked[0][0] - w as f32).abs() < 0.01);
    assert!((picked[0][1] - slot_ui::rest_y(h as f32)).abs() < 0.01);
    assert!(face(&app, textures(1003).side).is_some(), "carousel neighbours remain visible");

    app.apply(Action::GbaDown(Btn::Down));
    app.update(1.0);
    let box_face = face(&app, box_tex).unwrap();
    let cart = face(&app, cart_tex).unwrap();
    let box_w = box_face[1][0] - box_face[0][0];
    let box_h = box_face[3][1] - box_face[0][1];
    assert!(((cart[1][0] - cart[0][0]) / box_w - 60.0 / 125.0).abs() < 0.001);
    assert!(((cart[3][1] - cart[0][1]) / box_h - 35.0 / 125.0).abs() < 0.001);
    app.apply(Action::GbaDown(Btn::Up));
    app.update(1.0);
    equal_pose(picked, face(&app, cart_tex).unwrap());
    app.apply(Action::GbaDown(Btn::B));
    app.update(1.21);
    equal_pose(before, face(&app, box_tex).unwrap());
}

#[test]
fn open_box_controls_flip_zoom_pan_and_close_without_changing_shelf() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    open(&mut app);
    app.apply(Action::GbaDown(Btn::Down));
    app.update(1.0);
    app.apply(Action::GbaDown(Btn::A));
    app.update(0.08);
    assert!(face(&app, textures(1000).side).is_some());
    app.update(1.0);
    assert!(face(&app, textures(1000).back).is_some());
    app.apply(Action::NextShelf);
    app.apply(Action::GbaDown(Btn::R1));
    app.update(1.0);
    app.apply(Action::GbaUp(Btn::R1));
    let zoomed = face(&app, textures(1000).back).unwrap();
    app.apply(Action::ShelfLeft);
    app.apply(Action::GbaDown(Btn::Left));
    app.update(0.2);
    app.apply(Action::GbaUp(Btn::Left));
    let panned = face(&app, textures(1000).back).unwrap();
    assert!(panned[0][0] > zoomed[0][0]);
    app.apply(Action::GbaDown(Btn::B));
    app.update(1.0);
    assert!(matches!(app.phase(), Phase::Shelf));
    app.apply(Action::GbaDown(Btn::B));
    app.update(1.21);
    assert!(face(&app, textures(1000).front).is_some());
    assert_eq!(app.selected_stem(), Some("Alpha"));
}

#[test]
fn open_cart_keeps_picker_studio_and_quick_launch() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    open(&mut app);
    app.apply(Action::GbaDown(Btn::Start));
    assert!(app.core_picker().is_some());

    let mut app = setup(root.path());
    open(&mut app);
    app.apply(Action::SelectTap);
    assert!(app.studio().is_some());

    let mut app = setup(root.path());
    app.apply(Action::GbaDown(Btn::Start));
    app.update(1.21);
    assert!(matches!(app.phase(), Phase::Inserting { .. }));

    let mut app = setup(root.path());
    app.apply(Action::GbaDown(Btn::A));
    app.update(0.51);
    app.update(1.21);
    assert!(matches!(app.phase(), Phase::Inserting { .. }));
    assert!(app.starting_clean());
}

#[test]
fn missing_box_art_keeps_normal_cartridge_launch() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    write_carousel_view(root.path(), CarouselView::Boxes).unwrap();
    let mut app = common::boot(root.path());
    assert!(matches!(app.phase(), Phase::Shelf));
    app.apply(Action::GbaDown(Btn::A));
    app.apply(Action::GbaUp(Btn::A));
    assert!(matches!(app.phase(), Phase::Inserting { .. }));
}
