use slot_gfx::Draw;
use std::f32::consts::PI;

use crate::shelf::BoxFaceTextures;

struct Face {
    corners: [[f32; 2]; 4],
    depth: [f32; 4],
    tex: slot_gfx::TexId,
}

struct Projection {
    left: f32,
    right: f32,
    front: f32,
    height: f32,
    camera: f32,
    sin: f32,
    cos: f32,
}

impl Projection {
    fn new((width, height, depth): (f32, f32, f32), angle: f32) -> Self {
        let (sin, cos) = angle.clamp(0.0, PI).sin_cos();
        Self {
            left: -depth / 2.0,
            right: width - depth / 2.0,
            front: depth / 2.0,
            height,
            camera: width.max(height).max(depth) * 4.0,
            sin,
            cos,
        }
    }

    fn distance(&self, x: f32, z: f32) -> f32 {
        // The red pivot is halfway between front and back, half a depth from the spine.
        let rz = -x * self.sin + z * self.cos;
        self.camera - rz + self.front * (self.cos.abs() + self.sin)
    }

    fn point(&self, x: f32, y: f32, z: f32) -> [f32; 2] {
        let rx = x * self.cos + z * self.sin;
        let factor = self.camera / self.distance(x, z);
        [rx * factor, y * factor]
    }

    fn bounds(&self) -> (f32, f32) {
        let seam_front = self.point(self.left, 0.0, self.front)[0];
        let seam_back = self.point(self.left, 0.0, -self.front)[0];
        let edge = self.point(
            self.right,
            0.0,
            if self.cos >= 0.0 {
                self.front
            } else {
                -self.front
            },
        )[0];
        (
            seam_back.min(seam_front).min(edge),
            seam_back.max(seam_front).max(edge),
        )
    }
}

fn faces(textures: BoxFaceTextures, p: &Projection) -> Vec<Face> {
    let (left, right, front) = (p.left, p.right, p.front);
    let back = -front;
    let quad = |a: (f32, f32), b: (f32, f32), tex| Face {
        corners: [
            p.point(a.0, -p.height / 2.0, a.1),
            p.point(b.0, -p.height / 2.0, b.1),
            p.point(b.0, p.height / 2.0, b.1),
            p.point(a.0, p.height / 2.0, a.1),
        ],
        depth: [
            p.distance(a.0, a.1) / p.camera,
            p.distance(b.0, b.1) / p.camera,
            p.distance(b.0, b.1) / p.camera,
            p.distance(a.0, a.1) / p.camera,
        ],
        tex,
    };
    let mut result = Vec::with_capacity(2);
    if p.sin > 0.00001 {
        result.push(quad((left, back), (left, front), textures.side));
    }
    if p.cos > 0.00001 {
        result.push(quad((left, front), (right, front), textures.front));
    } else if p.cos < -0.00001 {
        // Back art is read from the free edge toward the spine.
        result.push(quad((right, back), (left, back), textures.back));
    }
    result
}

/// Projected width of a box rotated about its spine-side pivot.
pub fn box_rotation_width(size: (f32, f32, f32), angle: f32) -> f32 {
    let (min, max) = Projection::new(size, angle).bounds();
    max - min
}

/// Draws a box centered horizontally at `center`, with its pivot at `center[1]`.
pub fn draw_rotated_box(
    textures: BoxFaceTextures,
    size: (f32, f32, f32),
    angle: f32,
    center: [f32; 2],
    alpha: f32,
    out: &mut Vec<Draw>,
) {
    let projection = Projection::new(size, angle);
    let (min, max) = projection.bounds();
    let x = center[0] - (min + max) / 2.0;
    for face in faces(textures, &projection) {
        out.push(Draw::PerspectiveQuad {
            corners: face.corners.map(|p| [p[0] + x, p[1] + center[1]]),
            depth: face.depth,
            tex: face.tex,
            uv: [0.0, 0.0, 1.0, 1.0],
            alpha,
        });
    }
}
