//! Box View scene ownership and cartridge geometry.
use slot_gfx::Canvas;
use slot_ui::Millis;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Item {
    Manual,
    Cartridge,
    Box,
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxPose {
    pub center: [f32; 2],
    pub scale: f32,
    pub angle: f32,
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Scene {
    Carousel,
    FlippingBack { started: Millis, from: BoxPose },
    Back { zoom: f32, pan: [f32; 2] },
    FlippingFront { started: Millis, from: BoxPose },
    Opening { started: Millis },
    QuickOpening { started: Millis },
    QuickLaunched,
    QuickClosing { started: Millis },
    Open { item: Item },
    PreparingClose { started: Millis, angle: f32 },
    Closing { started: Millis },
}
pub type BoxItem = Item;
pub type BoxScene = Scene;

pub const MANUAL_PICKUP_SECONDS: f32 = 0.30;
pub const MANUAL_TURN_SECONDS: f32 = 0.50;
pub const MANUAL_DOUBLE_TAP_MS: Millis = 300;

#[derive(Clone, Debug)]
pub struct ManualTurn {
    pub from: usize,
    pub to: usize,
    pub elapsed: f32,
    pub from_spine: f32,
    pub to_spine: f32,
    pub put_down: bool,
}

#[derive(Clone, Debug)]
pub struct ManualReader {
    pub page: usize,
    pub count: usize,
    pub pickup: f32,
    pub closing: bool,
    pub zoom: f32,
    pub pan: [f32; 2],
    pub buttons: [bool; 6],
    pub resetting: bool,
    pub turn: Option<ManualTurn>,
    pub last_tap: Option<(bool, Millis)>,
}

impl ManualReader {
    pub fn new(count: usize) -> Self {
        Self {
            page: 0,
            count,
            pickup: 0.0,
            closing: false,
            zoom: 1.0,
            pan: [0.0; 2],
            buttons: [false; 6],
            resetting: false,
            turn: None,
            last_tap: None,
        }
    }

    pub fn spread(&self, page: usize) -> [Option<usize>; 2] {
        if page == 0 || page + 1 == self.count {
            [Some(page), None]
        } else {
            let left = 1 + ((page - 1) / 2) * 2;
            [Some(left), (left + 1 < self.count - 1).then_some(left + 1)]
        }
    }

    pub fn wants(&self) -> Vec<usize> {
        let mut pages = Vec::new();
        for page in [
            self.page.saturating_sub(2),
            self.page.saturating_sub(1),
            self.page,
            (self.page + 1).min(self.count - 1),
            (self.page + 2).min(self.count - 1),
        ] {
            for index in self.spread(page).into_iter().flatten() {
                if index != 0 && !pages.contains(&index) {
                    pages.push(index);
                }
            }
        }
        if let Some(turn) = &self.turn {
            for page in [turn.from, turn.to] {
                for index in self.spread(page).into_iter().flatten() {
                    if index != 0 && !pages.contains(&index) {
                        pages.push(index);
                    }
                }
            }
        }
        pages
    }
}

/// Returns the screen rect `[x, y, w, h]` of the cartridge in the fully-open
/// Box View, i.e. at `progress = 1.0` of `draw_open_box`.  Used as
/// `studio_from` when the studio is opened from Open Box View so that the
/// slide-in animation departs from the correct on-screen position.
pub fn open_box_cart_rect(canvas: Canvas, cw: f32, ch: f32) -> [f32; 4] {
    let x = canvas.w / 2.0 - cw / 2.0;
    let y = slot_ui::rest_y(canvas, ch);
    [x, y, cw, ch]
}

#[cfg(test)]
mod tests {
    use super::ManualReader;

    #[test]
    fn spreads_include_every_page_for_short_and_odd_manuals() {
        for count in 1..=11 {
            let mut reader = ManualReader::new(count);
            assert_eq!(reader.spread(0), [Some(0), None]);
            assert_eq!(reader.spread(count - 1), [Some(count - 1), None]);
            for page in 0..count {
                reader.page = page;
                assert!(reader.spread(page).contains(&Some(page)));
                assert!(reader.wants().len() <= 8);
            }
        }
    }
}
