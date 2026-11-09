//! Box View scene ownership and Box View-specific insert/eject pose.
use slot_gfx::{Canvas, Draw, OUT_H};
use slot_ui::{BoxFaceTextures, Millis, MOUTH_H};

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Item { Cartridge, Box }
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxPose {
    pub center: [f32; 2],
    pub scale: f32,
    pub angle: f32,
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Scene {
    Carousel, FlippingBack { started: Millis, from: BoxPose }, Back { zoom:f32, pan:[f32;2] },
    FlippingFront { started:Millis, from: BoxPose }, Opening { started:Millis, from_back:bool },
    Open { item:Item }, Closing { started:Millis },
}
pub type BoxItem=Item;
pub type BoxScene=Scene;

fn ease(t:f32)->f32 { let t=t.clamp(0.0,1.0); t*t*t*(t*(t*6.0-15.0)+10.0) }
fn travel(seat:f32,catch:f32)->f32 {
    let (ci,co,creep)=(0.42,0.62,0.03);
    if seat<ci { catch*ease(seat/ci) }
    else if seat<co { catch+creep*(seat-ci)/(co-ci) }
    else { let caught=catch+creep; caught+(1.0-caught)*ease((seat-co)/(1.0-co)) }
}
fn pose(canvas:Canvas,from:[f32;4],native:(f32,f32),seated:(f32,f32),seat:f32)->[f32;4] {
    let lip_y=OUT_H as f32-MOUTH_H;
    let seated_y=lip_y+6.0;
    let foot0=from[1]+from[3];
    let seated_foot=seated_y+seated.1;
    let catch=(lip_y-foot0)/(seated_foot-foot0);
    let t=travel(seat.clamp(0.0,1.0),catch);
    let k=(t/catch).min(1.0);
    let (w0,h0)=native;
    let (w,h)=(w0+(seated.0-w0)*k,h0+(seated.1-h0)*k);
    let centre=from[0]+from[2]/2.0;
    let x=centre+(canvas.w/2.0-centre)*k-w/2.0;
    let y=foot0+(seated_foot-foot0)*t-h;
    [x,y,w,h]
}
fn draw_box(canvas:Canvas,faces:BoxFaceTextures,size:(f32,f32),p:f32,insert:bool,out:&mut Vec<Draw>) {
    let t=ease(p);let shift=canvas.w*0.72*t*if insert {-1.0}else{1.0};
    let alpha=if insert {1.0-0.25*t}else{0.75+0.25*t};
    out.push(Draw::Tex{x:canvas.w*0.20-size.0/2.0+shift,y:(canvas.h-size.1)/2.0,w:size.0,h:size.1,tex:faces.back,alpha});
}
fn transition(canvas:Canvas,faces:BoxFaceTextures,box_size:(f32,f32),from:[f32;4],native:(f32,f32),seated:(f32,f32),seat:f32,insert:bool,out:&mut Vec<Draw>,draw:impl FnOnce([f32;4],&mut Vec<Draw>)) {
    let rect=pose(canvas,from,native,seated,seat);
    draw(rect,out);
    draw_box(canvas,faces,box_size,if insert {seat}else{1.0-seat},insert,out);
}
pub fn insert(canvas:Canvas,faces:BoxFaceTextures,box_size:(f32,f32),from:[f32;4],native:(f32,f32),seated:(f32,f32),seat:f32,out:&mut Vec<Draw>,draw:impl FnOnce([f32;4],&mut Vec<Draw>)) {
    transition(canvas,faces,box_size,from,native,seated,seat,true,out,draw)
}
pub fn eject(canvas:Canvas,faces:BoxFaceTextures,box_size:(f32,f32),from:[f32;4],native:(f32,f32),seated:(f32,f32),seat:f32,out:&mut Vec<Draw>,draw:impl FnOnce([f32;4],&mut Vec<Draw>)) {
    transition(canvas,faces,box_size,from,native,seated,seat,false,out,draw)
}
/// Draws the emulator picker over the Open Box View background.
///
/// NOTE: Due to Rust borrow constraints (the picker closure and the background
/// closure both need `&self` + `&mut out`), the actual Box View branching is
/// handled inline in `app.rs`.  This function documents the intended layering:
/// background first, then the vanilla picker on top.
pub fn emulator_picker(draw_background: impl FnOnce(), draw_picker: impl FnOnce()) {
    draw_background();
    draw_picker();
}

/// Draws the Cartridge Studio over the Open Box View background.
///
/// NOTE: Due to Rust borrow constraints the actual Box View branching is
/// handled inline in `app.rs`.  This function documents the intended layering:
/// background first, then the studio draw on top.
pub fn cartridge_studio(draw_background: impl FnOnce(), draw_studio: impl FnOnce()) {
    draw_background();
    draw_studio();
}

/// Returns the screen rect `[x, y, w, h]` of the cartridge in the fully-open
/// Box View, i.e. at `progress = 1.0` of `draw_open_box`.  Used as
/// `studio_from` when the studio is opened from Open Box View so that the
/// slide-in animation departs from the correct on-screen position.
pub fn open_box_cart_rect(canvas: Canvas, cw: f32, ch: f32) -> [f32; 4] {
    let x = canvas.w / 2.0 - cw / 2.0;
    let y = (canvas.h - ch) / 2.0;
    [x, y, cw, ch]
}
