use ratatui::style::Color;

pub struct Ashen {
    pub ash: Color,
    pub light_ash: Color,
    pub pale_ash: Color,
    pub deep_ash: Color,
    pub charcoal: Color,
    pub smoke: Color,
    pub ember: Color,
    pub ember_glow: Color,
    pub frost: Color,
    pub moss: Color,
    pub rust: Color,
    pub slate: Color,
    pub bone: Color,
    pub phantom: Color,
    pub stone: Color,
}

pub const ASHEN: Ashen = Ashen {
    ash: Color::Rgb(0xa0, 0xa0, 0xa6),
    light_ash: Color::Rgb(0xb8, 0xb8, 0xbc),
    pale_ash: Color::Rgb(0xc0, 0xc0, 0xc4),
    deep_ash: Color::Rgb(0x5c, 0x5c, 0x62),
    charcoal: Color::Rgb(0x2a, 0x2a, 0x2e),
    smoke: Color::Rgb(0x9c, 0x9c, 0xa2),
    ember: Color::Rgb(0xb0, 0x71, 0x56),
    ember_glow: Color::Rgb(0xc4, 0x82, 0x6a),
    frost: Color::Rgb(0x6a, 0x8a, 0x9a),
    moss: Color::Rgb(0x8a, 0x9a, 0x7a),
    rust: Color::Rgb(0xa0, 0x60, 0x50),
    slate: Color::Rgb(0x7a, 0x8a, 0x9a),
    bone: Color::Rgb(0xd8, 0xd8, 0xdc),
    phantom: Color::Rgb(0x46, 0x50, 0x54),
    stone: Color::Rgb(0x38, 0x3c, 0x40),
};

pub struct Theme {
    pub accent: Color,
    pub tool_bg: Color,
    pub page_bg: Color,
    pub user_bg: Color,
    pub selected_bg: Color,
    pub header_bg: Color,
    pub input_bg: Color,
    pub separator: Color,
}

pub const THEME: Theme = Theme {
    accent: Color::Rgb(0xb0, 0x71, 0x56),
    tool_bg: Color::Rgb(0x28, 0x2e, 0x28),
    page_bg: Color::Rgb(0x1a, 0x1a, 0x1e),
    user_bg: Color::Rgb(0x2e, 0x2e, 0x32),
    selected_bg: Color::Rgb(0x38, 0x38, 0x3c),
    header_bg: Color::Rgb(0x22, 0x22, 0x26),
    input_bg: Color::Rgb(0x1e, 0x1e, 0x22),
    separator: Color::Rgb(0x36, 0x36, 0x3a),
};

impl Theme {
    pub fn ashen() -> Self {
        Self {
            accent: ASHEN.ember,
            tool_bg: THEME.tool_bg,
            page_bg: THEME.page_bg,
            user_bg: THEME.user_bg,
            selected_bg: THEME.selected_bg,
            header_bg: THEME.header_bg,
            input_bg: THEME.input_bg,
            separator: THEME.separator,
        }
    }
}
