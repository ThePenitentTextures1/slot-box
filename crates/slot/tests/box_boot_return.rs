mod common;
use slot::app::{App, Phase};
use slot_input::{Action, Btn};
use slot_store::{write_carousel_view, CarouselView};
use slot_ui::{BoxFaceTextures, Draw, TexId};
const FRONT: TexId = TexId::from_raw(1000);
const FLAP: TexId = TexId::from_raw(4000);
const CART: TexId = TexId::from_raw(2000);
const MANUAL: TexId = TexId::from_raw(9000);
fn textures(flap: Option<TexId>) -> BoxFaceTextures {
    BoxFaceTextures {
        front: FRONT,
        side: TexId::from_raw(1001),
        back: TexId::from_raw(1002),
        flap,
        front_size: (100, 100),
        side_size: (20, 100),
        back_size: (100, 100),
        japanese: false,
        pivot: [120.0, 50.0],
        front_center: [170.0, 50.0],
    }
}
fn set_selected_art(app: &mut App, art: Option<BoxFaceTextures>) {
    let (shelf, index) = app.selected_box_seat();
    app.set_box_faces(shelf, index, art);
    app.set_cart_face(shelf, index, CART);
}
fn frame(app: &App) -> Vec<Draw> {
    let mut draws = Vec::new();
    app.draw(&mut draws);
    draws
}
fn face(app: &App, tex: TexId) -> Option<[[f32; 2]; 4]> {
    frame(app).into_iter().find_map(|draw| match draw {
        Draw::PerspectiveQuad {
            tex: t, corners, ..
        } if t == tex => Some(corners),
        Draw::Tex {
            tex: t, x, y, w, h, ..
        } if t == tex => Some([[x, y], [x + w, y], [x + w, y + h], [x, y + h]]),
        _ => None,
    })
}
fn assert_same_pose(a: [[f32; 2]; 4], b: [[f32; 2]; 4]) {
    for (a, b) in a.iter().flatten().zip(b.iter().flatten()) {
        assert!((a - b).abs() < 0.001, "{a} != {b}");
    }
}
fn finish_eject(app: &mut App) {
    for _ in 0..400 {
        if matches!(app.phase(), Phase::Shelf) {
            return;
        }
        app.update(0.01);
    }
    panic!("eject never reached the shelf");
}
#[test]
fn rebooted_box_launches_retrace_quick_start_and_later_launches_keep_their_context() {
    for quick in [false, true] {
        let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
        write_carousel_view(root.path(), CarouselView::Boxes).unwrap();
        let mut initial = common::boot(root.path());
        set_selected_art(&mut initial, Some(textures(Some(FLAP))));
        let opening = if quick {
            common::through(&mut initial, Btn::Start);
            initial.update(0.11);
            Some((
                face(&initial, FRONT).unwrap(),
                face(&initial, CART).unwrap(),
            ))
        } else {
            common::through(&mut initial, Btn::A);
            initial.update(1.78);
            common::through(&mut initial, Btn::A);
            None
        };
        if quick {
            initial.update(0.111);
        }
        assert!(matches!(initial.phase(), Phase::Inserting { .. }));
        initial.on_core_ready();
        initial.update(1.0);
        assert!(matches!(initial.phase(), Phase::Playing { .. }));
        drop(initial);
        let mut resumed = App::boot(root.path());
        assert!(matches!(
            resumed.phase(),
            Phase::Inserting { resumed: true, .. }
        ));
        set_selected_art(&mut resumed, Some(textures(None)));
        assert!(resumed
            .box_detail_want()
            .is_some_and(|(_, _, _, flap)| flap));
        assert!(resumed
            .box_prefetch_wants()
            .contains(&resumed.selected_box_seat()));
        set_selected_art(&mut resumed, Some(textures(Some(FLAP))));
        resumed.on_core_ready();
        resumed.update(0.1);
        assert!(matches!(resumed.phase(), Phase::Playing { .. }));
        resumed.apply(Action::Eject);
        finish_eject(&mut resumed);
        let cart_at_slot = face(&resumed, CART).unwrap();
        let cart = resumed.carts().find(|cart| cart.stem == "Alpha").unwrap();
        let [x, y, w, h] = slot_ui::aligned_rect(resumed.canvas(), cart.platform);
        assert_same_pose(
            cart_at_slot,
            [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
        );
        resumed.update(0.11);
        if let Some((box_pose, cart_pose)) = opening {
            assert_same_pose(box_pose, face(&resumed, FRONT).unwrap());
            assert_same_pose(cart_pose, face(&resumed, CART).unwrap());
        }
        assert!(frame(&resumed)
            .iter()
            .any(|draw| matches!(draw, Draw::Part { tex, .. } if *tex == FLAP)));
        assert!(face(&resumed, FRONT).is_some());
        resumed.update(0.25);
        common::through(&mut resumed, Btn::Right);
        assert_eq!(resumed.selected_stem(), Some("Beta"));
        common::through(&mut resumed, Btn::Left);
        assert_eq!(resumed.selected_stem(), Some("Alpha"));
        let cart = resumed
            .carts()
            .find(|cart| cart.stem == "Alpha")
            .unwrap()
            .clone();
        resumed.set_manual_cover(Some((cart, MANUAL)));
        common::through(&mut resumed, Btn::A);
        resumed.update(1.78);
        common::through(&mut resumed, Btn::A);
        assert!(matches!(resumed.phase(), Phase::Inserting { .. }));
        resumed.apply(Action::Eject);
        finish_eject(&mut resumed);
        assert!(
            face(&resumed, MANUAL).is_some(),
            "open-box launch must return to its contents"
        );
    }
}
#[test]
fn boot_without_box_art_uses_the_same_eject_as_cartridge_view() {
    let mut poses = Vec::new();
    for view in [CarouselView::Boxes, CarouselView::Cartridges] {
        let root = common::tmp_root_with_carts(&["Alpha", "Beta"]);
        write_carousel_view(root.path(), view).unwrap();
        slot_store::write_slot_state(
            root.path(),
            &slot_store::SlotState {
                cart: Some("Alpha".into()),
                clock_set: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut app = App::boot(root.path());
        set_selected_art(&mut app, None);
        app.on_core_ready();
        app.update(0.1);
        app.apply(Action::Eject);
        while app.seat() > 0.3 {
            app.update(0.01);
        }
        poses.push(face(&app, CART).unwrap());
        finish_eject(&mut app);
        common::through(&mut app, Btn::Right);
        assert_eq!(app.selected_stem(), Some("Beta"));
    }
    assert_same_pose(poses[0], poses[1]);
}
