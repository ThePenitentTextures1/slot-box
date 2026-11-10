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
    app.update(1.97);
}

fn frame(app: &App) -> Vec<Draw> {
    let mut draws = Vec::new();
    app.draw(&mut draws);
    draws
}

#[test]
fn box_waits_for_cartridge_to_be_put_down_before_rising() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    open(&mut app);
    let parked = face(&app, textures(1000).front).unwrap();
    let picked = face(&app, TexId::from_raw(2000)).unwrap();
    app.apply(Action::GbaDown(Btn::Down));
    app.update(0.01);
    equal_pose(parked, face(&app, textures(1000).front).unwrap());
    let shrinking = face(&app, TexId::from_raw(2000)).unwrap();
    assert!(shrinking[1][0] - shrinking[0][0] < picked[1][0] - picked[0][0]);
    app.update(0.03);
    equal_pose(parked, face(&app, textures(1000).front).unwrap());
    app.update(0.06);
    let raised = face(&app, textures(1000).front).unwrap();
    let down = face(&app, TexId::from_raw(2000)).unwrap();
    assert!(raised[0][1] < parked[0][1]);
    assert!(((down[1][0] - down[0][0]) / (raised[1][0] - raised[0][0]) - 60.0 / 125.0).abs() < 0.001);
}

#[test]
fn open_box_footer_covers_slot_box_and_status_before_legends() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    let plugged = TexId::from_raw(6000);
    let clock = TexId::from_raw(6001);
    let legends: Vec<_> = (0..5).map(|i| (TexId::from_raw(6010 + i), 70)).collect();
    app.set_plugged_face(plugged);
    app.set_shelf_clock_face(clock, 60);
    app.set_open_box_legends([legends.clone(), legends.clone()]);
    open(&mut app);
    let out = frame(&app);
    let index = |tex| out.iter().position(|d| match d {
        Draw::Tex { tex: t, .. } | Draw::Part { tex: t, .. }
        | Draw::PerspectiveQuad { tex: t, .. } => *t == tex,
        _ => false,
    }).unwrap();
    let bar = index(plugged);
    assert!(index(clock) < bar);
    assert!(index(textures(1000).front) < bar);
    assert!(bar < index(legends[0].0) && bar < index(legends[1].0));
    assert!(matches!(out[bar], Draw::Part { y, h, alpha: 1.0, .. }
        if y == slot_ui::OUT_H as f32 - slot_ui::MOUTH_H && h == slot_ui::MOUTH_H));
}

#[test]
fn footer_fades_in_before_the_box_moves_and_also_marks_inspection() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    let plugged = TexId::from_raw(6000);
    let legends: Vec<_> = (0..5).map(|i| (TexId::from_raw(6010 + i), 70)).collect();
    app.set_plugged_face(plugged);
    app.set_open_box_legends([legends.clone(), legends.clone()]);
    let before = face(&app, textures(1000).front).unwrap();
    common::through(&mut app, Btn::A);
    app.update(0.08);
    equal_pose(before, face(&app, textures(1000).front).unwrap());
    let out = frame(&app);
    assert!(out.iter().any(|d| matches!(d, Draw::Part { tex, alpha, .. }
        if *tex == plugged && (*alpha - 0.5).abs() < 0.02)));
    assert!(out.iter().any(|d| matches!(d, Draw::Tex { tex, alpha, .. }
        if *tex == legends[0].0 && (*alpha - 0.5).abs() < 0.02)));
    app.update(0.08);
    equal_pose(before, face(&app, textures(1000).front).unwrap());
    app.update(0.1);
    assert_ne!(before, face(&app, textures(1000).front).unwrap());

    let mut app = setup(root.path());
    app.set_plugged_face(plugged);
    app.set_open_box_legends([legends.clone(), legends.clone()]);
    app.apply(Action::GbaDown(Btn::A));
    common::through(&mut app, Btn::Select);
    app.update(0.08);
    assert!(frame(&app).iter().any(|d| matches!(d, Draw::Part { tex, alpha, .. }
        if *tex == plugged && (*alpha - 0.5).abs() < 0.02)));
    app.update(0.25);
    let out = frame(&app);
    let index = |tex| out.iter().position(|d| match d {
        Draw::Tex { tex: t, .. } | Draw::Part { tex: t, .. }
        | Draw::PerspectiveQuad { tex: t, .. } => *t == tex,
        _ => false,
    }).unwrap();
    assert!(index(textures(1000).back) < index(plugged));
    assert!(index(plugged) < index(legends[0].0));
    assert!(index(plugged) < index(legends[3].0));
    assert!(!out.iter().any(|d| matches!(d, Draw::Tex { tex, .. } if *tex == legends[1].0)));
    common::through(&mut app, Btn::Start);
    app.apply(Action::QuickMenu);
    common::through(&mut app, Btn::Y);
    app.update(0.6);
    assert!(matches!(app.phase(), Phase::Shelf));
    assert!(app.core_picker().is_none());
    assert!(!app.is_favorite(slot_store::Platform::Gba, "Alpha"));
    app.apply(Action::GbaDown(Btn::B));
    app.update(0.33);
    assert!(face(&app, textures(1000).front).is_some());
}

#[test]
fn picked_up_cartridges_are_in_front_of_the_parked_box() {
    for root in [
        common::tmp_root_with_carts(&["Alpha", "Beta"]),
        common::tmp_root_with_gb_carts(&["Alpha", "Beta"]),
        common::tmp_root_with_gbc_carts(&["Alpha", "Beta"]),
    ] {
        let mut app = setup(root.path());
        open(&mut app);
        let check_order = |app: &App, box_first: bool| {
            let out = frame(app);
            let box_at = out.iter().position(|d| matches!(d, Draw::PerspectiveQuad { tex, .. }
                if *tex == textures(1000).front)).unwrap();
            let cart_at = out.iter().position(|d| matches!(d, Draw::Tex { tex, .. }
                if *tex == TexId::from_raw(2000))).unwrap();
            assert_eq!(box_at < cart_at, box_first);
        };
        check_order(&app, true);
        app.apply(Action::GbaDown(Btn::Down));
        app.update(0.3);
        check_order(&app, true);
        app.apply(Action::GbaDown(Btn::B));
        app.update(0.3);
        check_order(&app, false);
    }
}

#[test]
fn inspecting_zoomed_box_can_pan_its_bottom_above_the_footer() {
    for carousel in [false, true] {
        for zoom_time in [0.13, 1.5] {
            let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
            let mut app = setup(root.path());
            if carousel {
                common::through(&mut app, Btn::Select);
                app.update(0.33);
            } else {
                open(&mut app);
                app.apply(Action::GbaDown(Btn::Down));
                app.update(1.0);
            }
            app.apply(Action::GbaDown(Btn::R1));
            app.update(zoom_time);
            app.apply(Action::GbaUp(Btn::R1));
            app.apply(Action::GbaDown(Btn::Down));
            app.update(5.0);
            app.apply(Action::GbaUp(Btn::Down));
            let tex = if carousel { textures(1000).back } else { textures(1000).front };
            let bottom = face(&app, tex).unwrap()[3][1];
            assert!(bottom <= slot_ui::OUT_H as f32 - slot_ui::MOUTH_H + 0.01,
                "box bottom {bottom} is covered, carousel={carousel}, zoom_time={zoom_time}");
        }
    }
}

#[test]
fn picker_keeps_carousel_and_box_without_duplicate_cartridge() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    open(&mut app);
    let before = frame(&app);
    let box_tex = textures(1000).front;
    let neighbour = textures(1003).side;
    let selected = TexId::from_raw(2000);
    let lid = TexId::from_raw(6101);
    app.set_core_board_faces(TexId::from_raw(6100), lid);
    app.apply(Action::GbaDown(Btn::Start));
    app.update(0.2);
    let out = frame(&app);
    assert!(out.iter().any(|d| matches!(d, Draw::PerspectiveQuad { tex, .. } if *tex == neighbour)));
    assert!(!out.iter().any(|d| matches!(d, Draw::Tex { tex, .. } if *tex == selected)));
    assert_eq!(out.iter().filter(|d| matches!(d, Draw::Turned { tex, .. } if *tex == lid)).count(), 1);
    let box_draw = |draws: &[Draw]| draws.iter().find_map(|d| match d {
        Draw::PerspectiveQuad { tex, corners, alpha, .. } if *tex == box_tex => Some((corners[0][1], *alpha)),
        _ => None,
    }).unwrap();
    let (y0, alpha0) = box_draw(&before);
    let (y1, alpha1) = box_draw(&out);
    assert!(y1 > y0 && alpha1 < alpha0);
    app.apply(Action::GbaDown(Btn::B));
    app.update(0.5);
    assert!(app.core_picker().is_none());
    let restored = frame(&app);
    let (y2, alpha2) = box_draw(&restored);
    assert!((y2 - y0).abs() < 0.001 && (alpha2 - alpha0).abs() < 0.001);
    assert_eq!(restored.iter().filter(|d| matches!(d, Draw::Tex { tex, .. } if *tex == selected)).count(), 1);
}

#[test]
fn studio_slide_retains_box_and_carousel_without_duplicate_cartridge() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    open(&mut app);
    app.apply(Action::SelectTap);
    app.update(0.05);
    let out = frame(&app);
    assert!(out.iter().any(|d| matches!(d, Draw::PerspectiveQuad { tex, .. } if *tex == textures(1000).front)));
    assert!(out.iter().any(|d| matches!(d, Draw::PerspectiveQuad { tex, .. } if *tex == textures(1003).side)));
    assert!(!out.iter().any(|d| matches!(d, Draw::Tex { tex, .. } if *tex == TexId::from_raw(2000))));
    assert_eq!(out.iter().filter(|d| matches!(d, Draw::Quad { tex, .. } if *tex == TexId::from_raw(2000))).count(), 1);
}

#[test]
fn insert_and_eject_use_the_vanilla_cartridge_pose() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut boxed = setup(root.path());
    open(&mut boxed);
    write_carousel_view(root.path(), CarouselView::Cartridges).unwrap();
    let mut vanilla = common::boot(root.path());
    for want in vanilla.face_wants() {
        vanilla.set_cart_face(want.shelf, want.index, TexId::from_raw(2000 + want.index));
    }
    for app in [&mut boxed, &mut vanilla] {
        app.apply(Action::GbaDown(Btn::A));
        app.apply(Action::GbaUp(Btn::A));
    }
    for dt in [0.0, 0.1, 0.2, 0.3] {
        boxed.update(dt);
        vanilla.update(dt);
        equal_pose(face(&boxed, TexId::from_raw(2000)).unwrap(), face(&vanilla, TexId::from_raw(2000)).unwrap());
    }
    for app in [&mut boxed, &mut vanilla] {
        app.apply(Action::Eject);
    }
    for dt in [0.0, 0.2, 0.3] {
        boxed.update(dt);
        vanilla.update(dt);
        equal_pose(face(&boxed, TexId::from_raw(2000)).unwrap(), face(&vanilla, TexId::from_raw(2000)).unwrap());
    }
}

#[test]
fn closing_a_backwards_box_flips_it_before_the_flap_opens() {
    for close in [Action::GbaDown(Btn::B), Action::ShelfLeft, Action::ShelfRight] {
        let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
        let mut app = setup(root.path());
        open(&mut app);
        app.apply(Action::GbaDown(Btn::Down));
        app.update(1.0);
        app.apply(Action::GbaDown(Btn::A));
        app.update(1.0);
        let back = face(&app, textures(1000).back).unwrap();
        app.apply(close);
        equal_pose(back, face(&app, textures(1000).back).unwrap());
        app.update(0.16);
        let out = frame(&app);
        assert!(out.iter().any(|d| matches!(d, Draw::PerspectiveQuad { tex, .. } if *tex == textures(1000).side)));
        assert!(!out.iter().any(|d| matches!(d, Draw::Part { tex, .. } if *tex == TexId::from_raw(4000))));
        app.update(0.17);
        assert!(face(&app, textures(1000).front).is_some());
        app.update(0.4);
        assert!(frame(&app).iter().any(|d| matches!(d, Draw::Part { tex, .. } if *tex == TexId::from_raw(4000))));
    }
}

#[test]
fn flap_unfolds_in_layers_and_folds_back_in_reverse() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    let flap = TexId::from_raw(4000);
    let cart = TexId::from_raw(2000);
    let box_front = textures(1000).front;
    let draw = |app: &App| {
        let mut out = Vec::new();
        app.draw(&mut out);
        out
    };
    let part = |draws: &[Draw], v: f32| {
        draws.iter().position(|draw| matches!(draw,
            Draw::Part { tex, uv, .. } if *tex == flap && (uv[1] - v).abs() < 0.0001
        ))
    };
    app.apply(Action::GbaDown(Btn::A));
    app.apply(Action::GbaUp(Btn::A));
    app.update(0.556);
    let folded = draw(&app);
    let lid = part(&folded, 2.0 / 3.0).unwrap();
    let outside = part(&folded, 0.0).unwrap();
    assert!(lid < outside);
    assert!(part(&folded, 1.0 / 3.0).is_none());
    if let (Draw::Part { y: ly, h: lh, .. }, Draw::Part { y: ty, h: th, uv, .. }) =
        (&folded[lid], &folded[outside])
    {
        assert!((ly - ty).abs() < 0.001 && (lh - th).abs() < 0.001);
        assert!((uv[3] - 1.0 / 3.0).abs() < 0.001);
    } else {
        panic!("expected lid and tab");
    }

    app.update(0.182);
    let before_turn = draw(&app);
    let outer = part(&before_turn, 0.0).unwrap();
    assert!(part(&before_turn, 1.0 / 3.0).is_none());
    if let Draw::Part { h, .. } = before_turn[outer] {
        assert!(h < 8.0);
    }
    app.update(0.004);
    let after_turn = draw(&app);
    let inner = part(&after_turn, 1.0 / 3.0).unwrap();
    assert!(part(&after_turn, 0.0).is_none());
    if let Draw::Part { h, .. } = after_turn[inner] {
        assert!(h < 8.0);
    }
    app.update(0.228);
    let opened = draw(&app);
    let lid = part(&opened, 2.0 / 3.0).unwrap();
    let inside = part(&opened, 1.0 / 3.0).unwrap();
    let cartridge = opened.iter().position(|d| matches!(d, Draw::Tex { tex, .. } if *tex == cart)).unwrap();
    let box_index = opened.iter().position(|d| matches!(d, Draw::PerspectiveQuad { tex, .. } if *tex == box_front)).unwrap();
    assert!(lid < inside && inside < cartridge && cartridge < box_index);
    assert!(part(&opened, 0.0).is_none());
    if let (Draw::Part { y: ly, h: lh, .. }, Draw::Part { y: ty, h: th, .. }) =
        (&opened[lid], &opened[inside])
    {
        assert!((ty + th - ly).abs() < 0.001 && (lh - th).abs() < 0.001);
    } else {
        panic!("expected lid and tab");
    }

    app.update(1.0);
    app.apply(Action::GbaDown(Btn::B));
    app.update(0.54);
    let closing_open = draw(&app);
    assert!(part(&closing_open, 1.0 / 3.0).is_some());
    app.update(0.86);
    let closing_folded = draw(&app);
    assert!(part(&closing_folded, 0.0).is_some());
    assert!(part(&closing_folded, 1.0 / 3.0).is_none());
}

#[test]
fn flap_lid_accelerates_smoothly_from_closed() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    common::through(&mut app, Btn::A);
    app.update(0.484);
    let lid_height = |app: &App| frame(app).iter().find_map(|draw| match draw {
        Draw::Part { tex, uv, h, .. }
            if *tex == TexId::from_raw(4000) && (uv[1] - 2.0 / 3.0).abs() < 0.0001 => Some(*h),
        _ => None,
    }).unwrap();
    let first = lid_height(&app);
    app.update(0.036);
    let second = lid_height(&app);
    app.update(0.036);
    let third = lid_height(&app);
    assert!(first > 0.0 && first < second && second < third);
    assert!(second - first < third - second);
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
    app.update(0.65);
    let mut scene = Vec::new();
    app.draw(&mut scene);
    assert!(scene.iter().any(|draw| matches!(draw, Draw::Part { tex, .. } if *tex == TexId::from_raw(4000))));
    app.update(1.13);
    let centered_small = face(&app, cart_tex).unwrap();
    let box_face = face(&app, box_tex).unwrap();
    assert!(((centered_small[1][0] - centered_small[0][0]) / (box_face[1][0] - box_face[0][0]) - 60.0 / 125.0).abs() < 0.001);
    let (_, native_h) = slot_ui::cart_box(slot_store::Platform::Gba);
    let cart_center = (centered_small[0][1] + centered_small[3][1]) / 2.0;
    assert!((cart_center - slot_ui::rest_y(native_h as f32) - native_h as f32 / 2.0).abs() < 0.01);
    app.update(0.19);
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
    app.update(1.81);
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
    app.update(1.81);
    assert!(face(&app, textures(1000).front).is_some());
    assert_eq!(app.selected_stem(), Some("Alpha"));
}

#[test]
fn open_cart_keeps_picker_studio_and_clean_launch() {
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
    open(&mut app);
    app.apply(Action::GbaDown(Btn::A));
    app.update(0.51);
    assert!(matches!(app.phase(), Phase::Inserting { .. }));
    assert!(app.starting_clean());
}

#[test]
fn boxed_carousel_keeps_collection_actions() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    app.set_heart_face(TexId::from_raw(7000), 16, 16);
    common::through(&mut app, Btn::Y);
    assert!(app.is_favorite(slot_store::Platform::Gba, "Alpha"));
    assert!(frame(&app).iter().any(|d| matches!(d, Draw::Tex { tex, .. } if *tex == TexId::from_raw(7000))));
    common::through(&mut app, Btn::Y);
    assert!(!app.is_favorite(slot_store::Platform::Gba, "Alpha"));
    common::through(&mut app, Btn::B);
    assert!(face(&app, textures(1000).front).is_some());
    app.apply(Action::GbaDown(Btn::A));
    app.apply(Action::GbaUp(Btn::A));
    app.update(1.97);
    assert!(matches!(app.phase(), Phase::Shelf));
    assert!(face(&app, TexId::from_raw(2000)).is_some());
    common::through(&mut app, Btn::Y);
    assert!(!app.is_favorite(slot_store::Platform::Gba, "Alpha"));
    app.apply(Action::GbaDown(Btn::A));
    app.apply(Action::GbaUp(Btn::A));
    assert!(matches!(app.phase(), Phase::Inserting { .. }));
}

#[test]
fn boxed_carousel_start_uses_auto_load_and_hold_a_starts_fresh() {
    for auto_load in [true, false] {
        let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
        let state = slot_store::SlotState { auto_load, ..Default::default() };
        slot_store::write_slot_state(root.path(), &state).unwrap();
        let mut app = setup(root.path());
        common::through(&mut app, Btn::Start);
        assert!(app.core_picker().is_none());
        app.update(1.97);
        assert!(matches!(app.phase(), Phase::Inserting { .. }));
        assert_eq!(app.starting_clean(), !auto_load);

        let mut app = setup(root.path());
        app.apply(Action::GbaDown(Btn::A));
        app.update(0.49);
        assert!(face(&app, textures(1000).front).is_some());
        app.update(0.02);
        app.apply(Action::GbaUp(Btn::A));
        app.update(1.97);
        assert!(matches!(app.phase(), Phase::Inserting { .. }));
        assert!(app.starting_clean());
    }
}

#[test]
fn boxed_game_can_be_removed_from_recents_on_the_carousel() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut state = slot_store::SlotState::default();
    state.show_recents = true;
    state.played(slot_store::Platform::Gba, "Beta");
    state.played(slot_store::Platform::Gba, "Alpha");
    slot_store::write_slot_state(root.path(), &state).unwrap();
    let mut app = setup(root.path());
    while app.shelf_platform_name() != Some("Recents") {
        app.apply(Action::PreviousShelf);
    }
    let selected = app.selected_stem().unwrap().to_string();
    common::through(&mut app, Btn::X);
    assert!(!slot_store::read_slot_state(root.path()).recents.iter().any(|r| r.stem == selected));
}

#[test]
fn remapped_collection_controls_work_on_the_box_carousel() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut keys = slot_store::CarouselKeys::default();
    keys.set(slot_store::CarouselAction::Favorite, slot_store::Binding::plain(slot_store::Device::R2));
    slot_store::write_carousel(root.path(), &keys).unwrap();
    let mut app = setup(root.path());
    common::through(&mut app, Btn::R2);
    assert!(app.is_favorite(slot_store::Platform::Gba, "Alpha"));
    common::through(&mut app, Btn::Right);
    assert_eq!(app.selected_stem(), Some("Beta"));
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
