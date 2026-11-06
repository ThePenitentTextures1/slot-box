//! Box View presentation dispatch. This module owns Box View animation entry points
//! while delegating to the existing renderer until view-specific choreography is added.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item { Box, Cartridge }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scene { Carousel, Back { zoom_q: u16, pan_x: i16, pan_y: i16 }, Opening { started_ms: u64, from_back: bool }, Open { selected: Item }, Closing { started_ms: u64 }, Inserting { started_ms: u64 }, Ejecting { started_ms: u64 }, Picker, Studio }

pub fn insert(draw_existing: impl FnOnce()) { draw_existing(); }
pub fn eject(draw_existing: impl FnOnce()) { draw_existing(); }
pub fn emulator_picker(draw_existing: impl FnOnce()) { draw_existing(); }
pub fn cartridge_studio(draw_existing: impl FnOnce()) { draw_existing(); }
