//! Box View scene ownership and insert/eject presentation.
use slot_gfx::Canvas;
use slot_ui::{BoxFaceTextures, Draw, Millis};

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Item { Cartridge, Box }
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Scene {
    Carousel,
    FlippingBack { started: Millis },
    Back { zoom: f32, pan: [f32; 2] },
    FlippingFront { started: Millis },
    Opening { started: Millis, from_back: bool },
    Open { item: Item },
    Closing { started: Millis },
}
pub type BoxItem = Item;
pub type BoxScene = Scene;

fn smooth(t: f32) -> f32 { let t=t.clamp(0.0,1.0); t*t*(3.0-2.0*t) }
fn box_back(canvas: Canvas, faces: BoxFaceTextures, size: (f32,f32), out: &mut Vec<Draw>) {
    let (w,h)=size;
    if w<=0.0 || h<=0.0 { return; }
    out.push(Draw::Tex{x:canvas.w*0.20-w/2.0,y:(canvas.h-h)/2.0,w,h,tex:faces.back,alpha:1.0});
}
/// Box-only insert entry point. Draws the box in its open-scene position and
/// delegates cartridge/slot choreography to the supplied renderer.
pub fn insert(canvas: Canvas, faces: BoxFaceTextures, size: (f32,f32), _progress: f32, out: &mut Vec<Draw>, draw_cart: impl FnOnce(&mut Vec<Draw>)) {
    draw_cart(out);
    box_back(canvas,faces,size,out);
}
/// Box-only eject entry point, isolated from vanilla Cartridge View drawing.
pub fn eject(canvas: Canvas, faces: BoxFaceTextures, size: (f32,f32), _progress: f32, out: &mut Vec<Draw>, draw_cart: impl FnOnce(&mut Vec<Draw>)) {
    box_back(canvas,faces,size,out);
    draw_cart(out);
}
pub fn emulator_picker(draw_existing: impl FnOnce()) { draw_existing(); }
pub fn cartridge_studio(draw_existing: impl FnOnce()) { draw_existing(); }
