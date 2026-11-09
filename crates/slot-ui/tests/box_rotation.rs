use std::f32::consts::{FRAC_PI_2, PI};

#[cfg(target_os = "macos")]
use slot_gfx::{Compositor, HeadlessSurface, OUT_H, OUT_W};
use slot_store::{Cart, Platform};
use slot_ui::{
    box_rotation_width, draw_rotated_box, label_colour, BoxFaceTextures, Canvas, Draw, Shelf, TexId,
};

fn textures(id: usize) -> BoxFaceTextures {
    BoxFaceTextures {
        front: TexId::from_raw(id),
        side: TexId::from_raw(id + 1),
        back: TexId::from_raw(id + 2),
        front_size: (100, 100),
        side_size: (20, 100),
        back_size: (100, 100),
        japanese: false,
        pivot: [120.0, 50.0],
        front_center: [170.0, 50.0],
    }
}

fn quad(draw: &Draw) -> (TexId, [[f32; 2]; 4]) {
    match *draw {
        Draw::PerspectiveQuad { tex, corners, .. } => (tex, corners),
        _ => panic!("expected a textured face"),
    }
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.001, "{a} != {b}");
}

#[test]
fn rotation_keeps_the_spine_joined_to_front_and_back() {
    let tex = textures(0);
    let size = (200.0, 200.0, 40.0);
    for angle in [PI / 4.0, PI * 3.0 / 4.0] {
        let mut out = Vec::new();
        draw_rotated_box(tex, size, angle, [300.0, 200.0], 1.0, &mut out);
        assert_eq!(out.len(), 2);
        let (side, spine) = quad(&out[0]);
        let (face, surface) = quad(&out[1]);
        assert_eq!(side, tex.side);
        if angle < FRAC_PI_2 {
            assert_eq!(face, tex.front);
            assert_eq!(spine[1], surface[0]);
            assert_eq!(spine[2], surface[3]);
            assert!(spine[0][0] < surface[0][0]);
        } else {
            assert_eq!(face, tex.back);
            assert_eq!(spine[0], surface[1]);
            assert_eq!(spine[3], surface[2]);
            assert!(spine[1][0] > surface[1][0]);
        }
        let min = out
            .iter()
            .flat_map(|d| quad(d).1)
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min);
        let max = out
            .iter()
            .flat_map(|d| quad(d).1)
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max);
        close(max - min, box_rotation_width(size, angle));
    }
}

#[test]
fn rotation_endpoints_show_the_expected_face_at_full_size() {
    let tex = textures(0);
    let size = (200.0, 180.0, 40.0);
    for (angle, expected, width) in [
        (0.0, tex.front, 200.0),
        (FRAC_PI_2, tex.side, 40.0),
        (PI, tex.back, 200.0),
    ] {
        let mut out = Vec::new();
        draw_rotated_box(tex, size, angle, [300.0, 200.0], 1.0, &mut out);
        assert_eq!(out.len(), 1);
        let (face, corners) = quad(&out[0]);
        assert_eq!(face, expected);
        close(corners[1][0] - corners[0][0], width);
        close(corners[3][1] - corners[0][1], size.1);
        close(box_rotation_width(size, angle), width);
    }
}

#[test]
fn faces_turn_away_at_the_perspective_adjusted_angle() {
    let tex = textures(0);
    for size in [(200.0_f32, 180.0_f32, 40.0_f32), (140.0, 220.0, 30.0)] {
        let camera = size.0.max(size.1).max(size.2) * 4.0;
        for step in 0..=1800 {
            let angle = PI * step as f32 / 1800.0;
            let (sin, cos) = angle.sin_cos();
            let camera_distance = camera + size.2 / 2.0 * (cos.abs() + sin);
            let mut out = Vec::new();
            draw_rotated_box(tex, size, angle, [300.0, 200.0], 1.0, &mut out);
            assert!(!out.is_empty(), "nothing visible at {angle}");
            for (id, facing) in [
                (tex.side, camera_distance * sin - size.2 / 2.0),
                (tex.front, camera_distance * cos - size.2 / 2.0),
                (tex.back, -camera_distance * cos - size.2 / 2.0),
            ] {
                if facing.abs() > 0.001 {
                    assert_eq!(
                        out.iter().any(|draw| quad(draw).0 == id),
                        facing > 0.0,
                        "wrong face at {angle} for {size:?}"
                    );
                }
            }
            for draw in &out {
                let (_, corners) = quad(draw);
                assert!(corners[1][0] > corners[0][0], "back-facing face at {angle}");
            }
        }
    }
}

#[test]
fn a_nearly_spine_on_box_does_not_draw_the_hidden_front_or_back() {
    let tex = textures(0);
    for degrees in [89.0_f32, 90.0, 91.0] {
        let mut out = Vec::new();
        draw_rotated_box(
            tex,
            (200.0, 180.0, 40.0),
            degrees.to_radians(),
            [300.0, 200.0],
            1.0,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(quad(&out[0]).0, tex.side);
    }
}

#[test]
fn face_depths_follow_the_same_projection_and_match_at_the_spine() {
    let tex = textures(0);
    let size = (200.0, 180.0, 40.0);
    for angle in [PI / 4.0, PI * 3.0 / 4.0] {
        let mut out = Vec::new();
        draw_rotated_box(tex, size, angle, [300.0, 200.0], 1.0, &mut out);
        let Draw::PerspectiveQuad { depth: side, .. } = out[0] else {
            panic!("expected perspective spine");
        };
        let Draw::PerspectiveQuad { depth: face, .. } = out[1] else {
            panic!("expected perspective face");
        };
        let far = 1.0 + size.0 * angle.sin() / (size.0 * 4.0);
        if angle < FRAC_PI_2 {
            close(side[1], face[0]);
            close(face[0], 1.0);
            close(face[1], far);
        } else {
            close(side[0], face[1]);
            close(face[1], 1.0);
            close(face[0], far);
        }
        close(face[0], face[3]);
        close(face[1], face[2]);
    }

    for angle in [0.0, FRAC_PI_2, PI] {
        let mut out = Vec::new();
        draw_rotated_box(tex, size, angle, [300.0, 200.0], 1.0, &mut out);
        let Draw::PerspectiveQuad { depth, .. } = out[0] else {
            panic!("expected perspective face");
        };
        for value in depth {
            close(value, 1.0);
        }
    }
}

#[test]
fn perspective_interpolation_preserves_artwork_on_both_triangles() {
    let size = (200.0, 180.0, 40.0);
    for angle in [PI / 4.0, PI * 3.0 / 4.0] {
        let mut out = Vec::new();
        draw_rotated_box(textures(0), size, angle, [300.0, 200.0], 1.0, &mut out);
        let Draw::PerspectiveQuad { corners, depth, .. } = out[1] else {
            panic!("expected perspective face");
        };
        let (sin, cos) = angle.sin_cos();
        let camera = size.0 * 4.0;
        let left = -size.2 / 2.0;
        let right = size.0 + left;
        let z = if angle < FRAC_PI_2 {
            size.2 / 2.0
        } else {
            -size.2 / 2.0
        };
        let x_at = |u: f32| {
            if angle < FRAC_PI_2 {
                left + u * size.0
            } else {
                right - u * size.0
            }
        };
        let project = |x: f32, y: f32| {
            let distance = camera - (-x * sin + z * cos) + size.2 / 2.0 * (cos.abs() + sin);
            [
                (x * cos + z * sin) * camera / distance,
                y * camera / distance,
            ]
        };
        let origin_x = corners[0][0] - project(x_at(0.0), -size.1 / 2.0)[0];
        let mut saw_affine_error = false;
        for (u, v, indices) in [(0.28, 0.25, [0, 1, 3]), (0.72, 0.75, [1, 2, 3])] {
            let p = project(x_at(u), (v - 0.5) * size.1);
            let point = [p[0] + origin_x, p[1] + 200.0];
            let [a, b, c] = indices.map(|i| corners[i]);
            let cross = |a: [f32; 2], b: [f32; 2]| a[0] * b[1] - a[1] * b[0];
            let sub = |a: [f32; 2], b: [f32; 2]| [a[0] - b[0], a[1] - b[1]];
            let area = cross(sub(b, a), sub(c, a));
            let wb = cross(sub(point, a), sub(c, a)) / area;
            let wc = cross(sub(b, a), sub(point, a)) / area;
            let weights = [1.0 - wb - wc, wb, wc];
            assert!(weights.iter().all(|w| *w >= 0.0));
            let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            let denom: f32 = (0..3).map(|i| weights[i] / depth[indices[i]]).sum();
            for (axis, expected) in [(0, u), (1, v)] {
                let corrected: f32 = (0..3)
                    .map(|i| weights[i] * uv[indices[i]][axis] / depth[indices[i]])
                    .sum::<f32>()
                    / denom;
                let affine: f32 = (0..3).map(|i| weights[i] * uv[indices[i]][axis]).sum();
                close(corrected, expected);
                saw_affine_error |= (affine - expected).abs() > 0.005;
            }
        }
        assert!(
            saw_affine_error,
            "test must expose the original affine distortion"
        );
    }
}

#[test]
fn spacing_matches_the_visible_geometry_throughout_the_rotation() {
    let tex = textures(0);
    for size in [(200.0, 180.0, 40.0), (140.0, 220.0, 30.0)] {
        for step in 0..=180 {
            let angle = PI * step as f32 / 180.0;
            let mut out = Vec::new();
            draw_rotated_box(tex, size, angle, [300.0, 200.0], 1.0, &mut out);
            let min = out
                .iter()
                .flat_map(|d| quad(d).1)
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min);
            let max = out
                .iter()
                .flat_map(|d| quad(d).1)
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max);
            close(max - min, box_rotation_width(size, angle));
        }
    }
}

#[test]
#[cfg(target_os = "macos")]
fn projected_front_spine_and_back_render_on_the_expected_sides() {
    let Ok(surface) = HeadlessSurface::new() else {
        return;
    };
    let Ok(mut compositor) = Compositor::new(&surface) else {
        return;
    };
    let front = compositor.create_texture(1, 1, &[255, 0, 0, 255]);
    let side = compositor.create_texture(1, 1, &[0, 255, 0, 255]);
    let back = compositor.create_texture(1, 1, &[0, 0, 255, 255]);
    let faces = BoxFaceTextures {
        front,
        side,
        back,
        ..textures(0)
    };
    for (angle, color) in [(PI / 4.0, [255, 0, 0]), (PI * 3.0 / 4.0, [0, 0, 255])] {
        let mut out = vec![Draw::Rect {
            x: 0.0,
            y: 0.0,
            w: OUT_W as f32,
            h: OUT_H as f32,
            colour: [0.0, 0.0, 0.0, 1.0],
        }];
        draw_rotated_box(
            faces,
            (200.0, 200.0, 40.0),
            angle,
            [300.0, 200.0],
            1.0,
            &mut out,
        );
        compositor.begin_frame();
        compositor.draw_list(&out);
        let frame = compositor.read_frame();
        for (draw, expected) in [(out[1], [0, 255, 0]), (out[2], color)] {
            let (_, corners) = quad(&draw);
            let x = (corners.iter().map(|p| p[0]).sum::<f32>() / 4.0) as usize;
            let y = (corners.iter().map(|p| p[1]).sum::<f32>() / 4.0) as usize;
            let offset = (y * OUT_W as usize + x) * 4;
            assert_eq!(&frame[offset..offset + 3], &expected);
        }
    }
}

fn shelf() -> Shelf {
    let mut shelf = Shelf::new(
        (0..8)
            .map(|i| Cart {
                platform: if i % 2 == 0 {
                    Platform::Gba
                } else {
                    Platform::Gb
                },
                stem: format!("Game {i}"),
                rom: format!("Game {i}.gba").into(),
                label: None,
                code: String::new(),
                shell: None,
                title: format!("Game {i}"),
            })
            .collect(),
    );
    shelf.set_box_view(true);
    for i in 0..8 {
        let mut faces = textures(i * 3);
        faces.japanese = i % 3 == 0;
        shelf.set_box_faces(i, Some(faces));
    }
    shelf
}

fn row(shelf: &Shelf) -> Vec<(usize, [[f32; 2]; 4])> {
    let mut out = Vec::new();
    shelf.draw_row(Canvas::FRAMED, None, 0.0, 0.0, 1.0, &mut out);
    let mut faces: Vec<_> = out
        .iter()
        .filter_map(|draw| match *draw {
            Draw::PerspectiveQuad { tex, corners, .. } => (0..24)
                .find(|i| TexId::from_raw(*i) == tex)
                .map(|i| (i, corners)),
            _ => None,
        })
        .collect();
    faces.sort_by_key(|face| face.0);
    faces
}

fn same_frame(a: &[(usize, [[f32; 2]; 4])], b: &[(usize, [[f32; 2]; 4])]) {
    assert_eq!(a.len(), b.len());
    for ((id_a, a), (id_b, b)) in a.iter().zip(b) {
        assert_eq!(id_a, id_b);
        for (a, b) in a.iter().zip(b) {
            close(a[0], b[0]);
            close(a[1], b[1]);
        }
    }
}

#[test]
fn navigation_and_reversal_do_not_swap_or_move_faces_on_the_input_frame() {
    let mut shelf = shelf();
    for press in [
        Shelf::right as fn(&mut Shelf),
        Shelf::left,
        Shelf::left,
        Shelf::right,
    ] {
        let before = row(&shelf);
        press(&mut shelf);
        same_frame(&before, &row(&shelf));
        for _ in 0..4 {
            shelf.update(1.0 / 60.0);
        }
    }
}

#[test]
fn carousel_rotates_both_items_through_their_joined_faces() {
    let mut shelf = shelf();
    shelf.right();
    shelf.update(0.06);
    let faces = row(&shelf);
    for id in [0, 3] {
        assert!(faces.iter().any(|(face, _)| *face == id));
        assert!(faces.iter().any(|(face, _)| *face == id + 1));
    }
}

#[test]
fn a_missing_box_uses_a_continuously_sized_cartridge() {
    let mut shelf = shelf();
    shelf.set_box_faces(1, None);
    let cart_rects = |shelf: &Shelf| {
        let mut out = Vec::new();
        shelf.draw_row(Canvas::FRAMED, None, 0.0, 0.0, 1.0, &mut out);
        let rgb = label_colour("Game 1");
        let mut rects: Vec<_> = out
            .into_iter()
            .filter_map(|draw| match draw {
                Draw::Rect { x, y, w, h, colour }
                    if (0..3).all(|c| (colour[c] * 255.0).round() as u8 == rgb[c]) =>
                {
                    Some([x, y, w, h])
                }
                _ => None,
            })
            .collect();
        rects.sort_by(|a, b| a[0].total_cmp(&b[0]));
        rects
    };
    let before = cart_rects(&shelf);
    assert!(!before.is_empty());
    shelf.right();
    let after = cart_rects(&shelf);
    assert_eq!(before.len(), after.len());
    for (a, b) in before.iter().zip(&after) {
        for (a, b) in a.iter().zip(b) {
            close(*a, *b);
        }
        shelf.update(0.06);
        assert_ne!(after, cart_rects(&shelf));
    }
}
