use ratatui::style::Color;

pub struct Ashen {
    pub pale_ash: Color,
    pub deep_ash: Color,
    pub charcoal: Color,
    pub smoke: Color,
    pub ember: Color,
    pub ember_glow: Color,
    pub frost: Color,
    pub moss: Color,
    pub slate: Color,
    pub bone: Color,
    pub stone: Color,
    pub whisper: Color,
}

pub const ASHEN: Ashen = Ashen {
    pale_ash: Color::Rgb(0xc0, 0xc0, 0xc4),
    deep_ash: Color::Rgb(0x5c, 0x5c, 0x62),
    charcoal: Color::Rgb(0x2a, 0x2a, 0x2e),
    smoke: Color::Rgb(0x9c, 0x9c, 0xa2),
    ember: Color::Rgb(0xb0, 0x71, 0x56),
    ember_glow: Color::Rgb(0xc4, 0x82, 0x6a),
    frost: Color::Rgb(0x6a, 0x8a, 0x9a),
    moss: Color::Rgb(0x8a, 0x9a, 0x7a),
    slate: Color::Rgb(0x7a, 0x8a, 0x9a),
    bone: Color::Rgb(0xd8, 0xd8, 0xdc),
    stone: Color::Rgb(0x38, 0x3c, 0x40),
    whisper: Color::Rgb(0x4a, 0x4a, 0x50),
};

pub struct Theme {
    pub accent: Color,
    pub header_bg: Color,
    pub input_bg: Color,
    pub page_bg: Color,
    pub separator: Color,
}

pub const THEME: Theme = Theme {
    accent: Color::Rgb(0xb0, 0x71, 0x56),
    page_bg: Color::Rgb(0x1a, 0x1a, 0x1e),
    header_bg: Color::Rgb(0x22, 0x22, 0x26),
    input_bg: Color::Rgb(0x1e, 0x1e, 0x22),
    separator: Color::Rgb(0x36, 0x36, 0x3a),
};
