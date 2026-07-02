//! MusicBee-flavoured styling for the iced UI. Colors are lifted directly
//! from the original HTML/CSS skin so the native app reads as the same
//! application, just without the webview.

use iced::border::Radius;
use iced::widget::{button, container, scrollable, text_input};
use iced::{Background, Border, Color, Gradient, Shadow, Vector};

// ---- Palette (from the original styles.css :root) -----------------------
pub fn rgb(hex: u32) -> Color {
    Color::from_rgb8(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

pub const FACE: u32 = 0x2E2E2E;
pub const PANEL: u32 = 0x242424;
pub const BORDER: u32 = 0x4A4A4A;
pub const BORDER_DK: u32 = 0x171717;
pub const TEXT: u32 = 0xE6E6E6;
pub const TEXT_DIM: u32 = 0xA8A8A8;
pub const SEL: u32 = 0x3E5D7A;
pub const SEL_BORDER: u32 = 0x7EA4C8;
pub const HOVER: u32 = 0x39424A;
pub const GRID_ALT: u32 = 0x2B2B2B;
pub const GROUP_BG: u32 = 0x303840;
pub const GROUP_BD: u32 = 0x4B5966;
pub const ACCENT: u32 = 0x8AB4E0;
pub const ACCENT_2: u32 = 0xB1D0F0;
pub const PLAYING: u32 = 0x2F4A63;

fn linear(from: u32, to: u32) -> Background {
    Background::Gradient(Gradient::Linear(
        iced::gradient::Linear::new(std::f32::consts::FRAC_PI_2)
            .add_stop(0.0, rgb(from))
            .add_stop(1.0, rgb(to)),
    ))
}

// ---- Surfaces -----------------------------------------------------------
pub fn root(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgb(FACE))),
        text_color: Some(rgb(TEXT)),
        ..Default::default()
    }
}

pub fn titlebar(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(linear(0x4B4B4B, 0x2A2A2A)),
        text_color: Some(rgb(0xF0F0F0)),
        border: Border {
            color: rgb(0x101010),
            width: 0.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

pub fn contextbar(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(linear(0x3D3D3D, 0x2D2D2D)),
        text_color: Some(rgb(TEXT)),
        border: Border {
            color: rgb(BORDER_DK),
            width: 1.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

pub fn sidebar(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgb(PANEL))),
        text_color: Some(rgb(TEXT)),
        border: Border {
            color: rgb(BORDER_DK),
            width: 1.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

pub fn content(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgb(FACE))),
        text_color: Some(rgb(TEXT)),
        ..Default::default()
    }
}

pub fn grid_header(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(linear(0x454545, 0x323232)),
        text_color: Some(rgb(ACCENT_2)),
        border: Border {
            color: rgb(0x1B1B1B),
            width: 1.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

pub fn statusbar(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(linear(0x343434, 0x282828)),
        text_color: Some(rgb(TEXT_DIM)),
        border: Border {
            color: rgb(BORDER_DK),
            width: 1.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

pub fn playerbar(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(linear(0x3F3F3F, 0x292929)),
        text_color: Some(rgb(TEXT)),
        border: Border {
            color: rgb(0x101010),
            width: 1.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

pub fn now_playing_panel(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgb(PANEL))),
        text_color: Some(rgb(TEXT)),
        border: Border {
            color: rgb(BORDER_DK),
            width: 1.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

// Track / list row backgrounds. `kind`: 0 = even, 1 = odd (alt), 2 = selected,
// 3 = currently playing.
pub fn row(kind: u8) -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| {
        let (bg, border) = match kind {
            2 => (rgb(SEL), rgb(SEL_BORDER)),
            3 => (rgb(PLAYING), rgb(SEL_BORDER)),
            1 => (rgb(GRID_ALT), rgb(GRID_ALT)),
            _ => (rgb(FACE), rgb(FACE)),
        };
        container::Style {
            background: Some(Background::Color(bg)),
            text_color: Some(rgb(TEXT)),
            border: Border {
                color: border,
                width: if kind >= 2 { 1.0 } else { 0.0 },
                radius: Radius::from(0.0),
            },
            ..Default::default()
        }
    }
}

pub fn group_header(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgb(GROUP_BG))),
        text_color: Some(rgb(ACCENT_2)),
        border: Border {
            color: rgb(GROUP_BD),
            width: 1.0,
            radius: Radius::from(2.0),
        },
        ..Default::default()
    }
}

pub fn art_tile(color: Color) -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: Some(Background::Color(color)),
        text_color: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.85)),
        border: Border {
            color: rgb(BORDER_DK),
            width: 1.0,
            radius: Radius::from(3.0),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.6),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 4.0,
        },
        ..Default::default()
    }
}

pub fn status_pill(connected: bool, error: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| {
        let (bg, fg, bd) = if !connected {
            (rgb(0x3A2420), rgb(0xFFD3C8), rgb(0x5A241D))
        } else if error {
            (rgb(0x3A3420), rgb(0xF0E6C8), rgb(0x5A4D1D))
        } else {
            (rgb(0x223322), rgb(0xC8F0C8), rgb(0x2C4A2C))
        };
        container::Style {
            background: Some(Background::Color(bg)),
            text_color: Some(fg),
            border: Border {
                color: bd,
                width: 1.0,
                radius: Radius::from(2.0),
            },
            ..Default::default()
        }
    }
}

// ---- Buttons ------------------------------------------------------------
fn base_btn(bg: Background, fg: Color, bd: Color, radius: f32) -> button::Style {
    button::Style {
        background: Some(bg),
        text_color: fg,
        border: Border {
            color: bd,
            width: 1.0,
            radius: Radius::from(radius),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn nav_tab(status: button::Status, active: bool, accent: Color) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered);
    if active {
        button::Style {
            border: Border {
                color: accent,
                width: 1.0,
                radius: Radius::from(5.0),
            },
            ..base_btn(
                Background::Color(rgb(PANEL)),
                accent,
                accent,
                5.0,
            )
        }
    } else {
        let bg = if hovered {
            Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08))
        } else {
            Background::Color(Color::TRANSPARENT)
        };
        button::Style {
            border: Border {
                color: if hovered { rgb(BORDER) } else { Color::TRANSPARENT },
                width: 1.0,
                radius: Radius::from(5.0),
            },
            ..base_btn(bg, rgb(0xE8E8E8), Color::TRANSPARENT, 5.0)
        }
    }
}

pub fn toolbar_btn(status: button::Status, active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let bg = if active || hovered {
        linear(0x565656, 0x3C4650)
    } else {
        linear(0x4C4C4C, 0x343434)
    };
    let bd = if active || hovered { rgb(0x6E8193) } else { rgb(BORDER_DK) };
    base_btn(bg, if active { rgb(ACCENT_2) } else { rgb(TEXT) }, bd, 2.0)
}

pub fn transport_btn(status: button::Status, primary: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let bg = if primary {
        linear(0x5A6E86, 0x39506A)
    } else if hovered {
        linear(0x565656, 0x3C4650)
    } else {
        linear(0x474747, 0x303030)
    };
    let bd = if primary { rgb(SEL_BORDER) } else { rgb(BORDER_DK) };
    base_btn(bg, rgb(if primary { 0xFFFFFF } else { 0xECECEC }), bd, 18.0)
}

pub fn toggle_btn(status: button::Status, active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let bg = if active {
        linear(0x39506A, 0x2A3C4F)
    } else if hovered {
        linear(0x4A4A4A, 0x333333)
    } else {
        linear(0x3C3C3C, 0x2A2A2A)
    };
    let fg = if active { rgb(ACCENT_2) } else { rgb(TEXT_DIM) };
    let bd = if active { rgb(SEL_BORDER) } else { rgb(BORDER_DK) };
    base_btn(bg, fg, bd, 3.0)
}

pub fn window_btn(status: button::Status, close: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let bg = if hovered {
        if close {
            Background::Color(rgb(0xC42B1C))
        } else {
            Background::Color(rgb(0x4B5966))
        }
    } else {
        Background::Color(Color::TRANSPARENT)
    };
    base_btn(bg, rgb(0xECECEC), Color::TRANSPARENT, 0.0)
}

// A clickable list/tree row used in the sidebars (artists, genres, tree).
pub fn list_item(status: button::Status, selected: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let bg = if selected {
        Background::Color(rgb(SEL))
    } else if hovered {
        Background::Color(rgb(HOVER))
    } else {
        Background::Color(Color::TRANSPARENT)
    };
    let fg = if selected { rgb(0xFFFFFF) } else { rgb(TEXT) };
    let bd = if selected {
        rgb(SEL_BORDER)
    } else if hovered {
        rgb(0x586572)
    } else {
        Color::TRANSPARENT
    };
    base_btn(bg, fg, bd, 2.0)
}

// Transparent button used to wrap a track row (the row container does the
// visible styling, the button only provides click handling).
pub fn bare_btn(_t: &iced::Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: rgb(TEXT),
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

// ---- Inputs -------------------------------------------------------------
pub fn search_input(_t: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(rgb(0x1E1E1E)),
        border: Border {
            color: if focused { rgb(ACCENT) } else { rgb(BORDER_DK) },
            width: 1.0,
            radius: Radius::from(2.0),
        },
        icon: rgb(TEXT_DIM),
        placeholder: rgb(0x808080),
        value: rgb(TEXT),
        selection: rgb(SEL),
    }
}

// ---- Slider (volume / seek) ---------------------------------------------
pub fn slider_style(_t: &iced::Theme, _status: iced::widget::slider::Status) -> iced::widget::slider::Style {
    use iced::widget::slider::{Handle, HandleShape, Rail};
    let filled = Background::Gradient(Gradient::Linear(
        iced::gradient::Linear::new(std::f32::consts::FRAC_PI_2)
            .add_stop(0.0, rgb(0x4E89D8))
            .add_stop(1.0, rgb(0x2A6BC4)),
    ));
    iced::widget::slider::Style {
        rail: Rail {
            backgrounds: (filled, Background::Color(rgb(0x1E1E1E))),
            width: 6.0,
            border: Border {
                color: rgb(0x141414),
                width: 1.0,
                radius: Radius::from(2.0),
            },
        },
        handle: Handle {
            shape: HandleShape::Rectangle {
                width: 10,
                border_radius: Radius::from(2.0),
            },
            background: linear(0xBEBEBE, 0x6B6B6B),
            border_width: 1.0,
            border_color: rgb(0x111111),
        },
    }
}

// ---- Scrollable ---------------------------------------------------------
pub fn scroller(_t: &iced::Theme, _status: scrollable::Status) -> scrollable::Style {
    let rail = scrollable::Rail {
        background: Some(Background::Color(rgb(0x252525))),
        border: Border {
            color: rgb(BORDER_DK),
            width: 0.0,
            radius: Radius::from(0.0),
        },
        scroller: scrollable::Scroller {
            background: Background::Color(rgb(0x4B4B4B)),
            border: Border {
                color: rgb(BORDER_DK),
                width: 1.0,
                radius: Radius::from(2.0),
            },
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(rgb(0x4B4B4B)),
            border: Border {
                color: rgb(BORDER_DK),
                width: 1.0,
                radius: Radius::from(2.0),
            },
            shadow: Shadow::default(),
            icon: rgb(TEXT),
        },
    }
}
