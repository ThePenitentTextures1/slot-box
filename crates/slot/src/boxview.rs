//! Box View scene ownership and cartridge geometry.
use slot_gfx::Canvas;
use slot_ui::Millis;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Item { Manual, Cartridge, Box }
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxPose {
    pub center: [f32; 2],
    pub scale: f32,
    pub angle: f32,
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Scene {
    Carousel, FlippingBack { started: Millis, from: BoxPose }, Back { zoom:f32, pan:[f32;2] },
    FlippingFront { started:Millis, from: BoxPose }, Opening { started:Millis },
    QuickOpening { started:Millis },
    Open { item:Item }, PreparingClose { started: Millis, angle: f32 }, Closing { started:Millis },
}
pub type BoxItem=Item;
pub type BoxScene=Scene;

/// Returns the screen rect `[x, y, w, h]` of the cartridge in the fully-open
/// Box View, i.e. at `progress = 1.0` of `draw_open_box`.  Used as
/// `studio_from` when the studio is opened from Open Box View so that the
/// slide-in animation departs from the correct on-screen position.
pub fn open_box_cart_rect(canvas: Canvas, cw: f32, ch: f32) -> [f32; 4] {
    let x = canvas.w / 2.0 - cw / 2.0;
    let y = slot_ui::rest_y(ch);
    [x, y, cw, ch]
}
