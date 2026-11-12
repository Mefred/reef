//! Dark green palette for Reef's native desktop shell.

use wgpui::{rgb, Hsla};

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn background() -> Hsla {
    color(0x0b1210)
}

pub fn sidebar() -> Hsla {
    color(0x0e1814)
}

pub fn panel() -> Hsla {
    color(0x121e19)
}

pub fn panel_alt() -> Hsla {
    color(0x17251f)
}

pub fn panel_border() -> Hsla {
    color(0x263a31)
}

pub fn text() -> Hsla {
    color(0xe4ede8)
}

pub fn text_muted() -> Hsla {
    color(0x8aa398)
}

pub fn text_faint() -> Hsla {
    color(0x61786d)
}

pub fn accent() -> Hsla {
    color(0x36d8a2)
}

pub fn accent_strong() -> Hsla {
    color(0x8ff0c5)
}

pub fn accent_dim() -> Hsla {
    color(0x1f6f54)
}

pub fn input_bg() -> Hsla {
    color(0x0a1310)
}

pub fn input_border() -> Hsla {
    color(0x2b4037)
}

pub fn danger() -> Hsla {
    color(0xf3a2a7)
}

pub fn warning() -> Hsla {
    color(0xe4bd79)
}

pub fn terminal_bg() -> Hsla {
    color(0x08100d)
}

pub fn selection() -> Hsla {
    color(0x1b332a)
}

/// Translucent scrim used behind modal dialogs. Content behind it remains
/// visible but is clearly inactive.
pub fn backdrop() -> Hsla {
    wgpui::rgba(0x05100cd9).into()
}

pub fn hover() -> Hsla {
    color(0x1d2e27)
}
