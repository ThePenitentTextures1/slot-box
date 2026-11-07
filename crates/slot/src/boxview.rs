//! Box View scene ownership and Box View-only cartridge transitions.
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

fn ease(t:f32)->f32 { let t=t.clamp(0.0,1.0); t*t*(3.0-2.0*t) }
fn draw_back(canvas:Canvas,faces:BoxFaceTextures,size:(f32,f32),progress:f32,insert:bool,out:&mut Vec<Draw>) {
    let t=ease(progress);
    let shift=canvas.w*0.72*t*if insert {-1.0} else {1.0};
    let x=canvas.w*0.20-size.0/2.0+shift;
    let alpha=if insert {1.0-0.25*t} else {0.75+0.25*t};
    if size.0>0.0 && size.1>0.0 { out.push(Draw::Tex{x,y:(canvas.h-size.1)/2.0,w:size.0,h:size.1,tex:faces.back,alpha}); }
}
fn draw_cart_transition(canvas:Canvas,faces:BoxFaceTextures,box_size:(f32,f32),from:[f32;4],to:[f32;4],progress:f32,insert:bool,out:&mut Vec<Draw>,draw:impl FnOnce([f32;4],&mut Vec<Draw>)) {
    let t=ease(progress);
    let rect=[from[0]+(to[0]-from[0])*t,from[1]+(to[1]-from[1])*t,from[2]+(to[2]-from[2])*t,from[3]+(to[3]-from[3])*t];
    draw(rect,out);
    draw_back(canvas,faces,box_size,progress,insert,out);
}
/// Box View insert: animate only the selected cartridge from the open-box position to the slot.
pub fn insert(canvas:Canvas,faces:BoxFaceTextures,box_size:(f32,f32),from:[f32;4],to:[f32;4],progress:f32,out:&mut Vec<Draw>,draw:impl FnOnce([f32;4],&mut Vec<Draw>)) {
    draw_cart_transition(canvas,faces,box_size,from,to,progress,true,out,draw);
}
/// Box View eject: reverse the selected cartridge motion back to the open box.
pub fn eject(canvas:Canvas,faces:BoxFaceTextures,box_size:(f32,f32),from:[f32;4],to:[f32;4],progress:f32,out:&mut Vec<Draw>,draw:impl FnOnce([f32;4],&mut Vec<Draw>)) {
    draw_cart_transition(canvas,faces,box_size,from,to,progress,false,out,draw);
}
pub fn emulator_picker(draw_existing:impl FnOnce()){draw_existing();}
pub fn cartridge_studio(draw_existing:impl FnOnce()){draw_existing();}
