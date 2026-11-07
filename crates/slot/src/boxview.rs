//! Box View scene ownership and Box View-specific insert/eject pose.
use slot_gfx::{Canvas, Draw, OUT_H};
use slot_ui::{BoxFaceTextures, Millis, MOUTH_H};

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Item { Cartridge, Box }
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Scene {
    Carousel, FlippingBack { started: Millis }, Back { zoom:f32, pan:[f32;2] },
    FlippingFront { started:Millis }, Opening { started:Millis, from_back:bool },
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
pub fn emulator_picker(draw:impl FnOnce()){draw()}
pub fn cartridge_studio(draw:impl FnOnce()){draw()}
