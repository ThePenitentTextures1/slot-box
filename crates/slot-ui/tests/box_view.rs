mod common;

use slot::app::App;
use slot_input::{Action, Btn};
use slot_store::{write_carousel_view, CarouselView};
use slot_ui::{BoxFaceTextures, Draw, TexId};

fn textures(id: usize) -> BoxFaceTextures {
    BoxFaceTextures {
        front: TexId::from_raw(id),
        side: TexId::from_raw(id + 1),
        back: TexId::from_raw(id + 2),
        flap: None,
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
        app.set_box_faces(want.shelf, want.index, Some(textures(1000 + want.index * 3)));
    }
    app
}

fn face(app: &App, tex: TexId) -> Option<[[f32; 2]; 4]> {
    let mut out = Vec::new();
    app.draw(&mut out);
    out.iter().filter_map(|draw| match *draw {
        Draw::Quad { tex: t, corners, .. } if t == tex => Some(corners),
        Draw::Tex { tex: t, x, y, w, h, .. } if t == tex => {
            Some([[x, y], [x + w, y], [x + w, y + h], [x, y + h]])
        }
        _ => None,
    }).min_by(|a, b| {
        let distance = |q: &[[f32; 2]; 4]| ((q[0][0] + q[1][0]) / 2.0 - app.canvas().w / 2.0).abs();
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
    assert!(app.shed_box_faces().is_empty(), "the flip evicted visible neighbours");

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
fn flipping_back_from_zoom_preserves_the_initial_pose() {
    let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
    let mut app = setup(root.path());
    let tex = textures(1000);
    app.apply(Action::GbaDown(Btn::L2));
    app.update(0.33);
    app.apply(Action::GbaDown(Btn::L1));
    let zoomed = face(&app, tex.back).unwrap();
    app.apply(Action::GbaDown(Btn::B));
    equal_pose(zoomed, face(&app, tex.back).unwrap());
}
