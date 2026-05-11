use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub accent: Color,
    pub bright: Color,
    pub dim1: Color,
    pub dim2: Color,
    pub dim3: Color,
    pub bar: Color,
}

impl Theme {
    pub fn catppuccin_mocha() -> Self {
        Self {
            bg: Color::Reset, // Fondo transparente para Hyprland
            accent: hex_to_color("#89b4fa"),
            bright: hex_to_color("#cdd6f4"),
            dim1: hex_to_color("#bac2de"),
            dim2: hex_to_color("#7f849c"),
            dim3: hex_to_color("#45475a"),
            bar: hex_to_color("#89b4fa"),
        }
    }

    pub fn gruvbox_dark() -> Self {
        Self {
            bg: Color::Reset,
            accent: hex_to_color("#fabd2f"), // Yellow
            bright: hex_to_color("#ebdbb2"),
            dim1: hex_to_color("#a89984"),
            dim2: hex_to_color("#7c6f64"),
            dim3: hex_to_color("#504945"),
            bar: hex_to_color("#fabd2f"),
        }
    }

    pub fn tokyo_night() -> Self {
        Self {
            bg: Color::Reset,
            accent: hex_to_color("#7aa2f7"),
            bright: hex_to_color("#c0caf5"),
            dim1: hex_to_color("#9aa5ce"),
            dim2: hex_to_color("#565f89"),
            dim3: hex_to_color("#414868"),
            bar: hex_to_color("#7aa2f7"),
        }
    }

    pub fn nord() -> Self {
        Self {
            bg: Color::Reset,
            accent: hex_to_color("#88c0d0"),
            bright: hex_to_color("#eceff4"),
            dim1: hex_to_color("#d8dee9"),
            dim2: hex_to_color("#4c566a"),
            dim3: hex_to_color("#434c5e"),
            bar: hex_to_color("#88c0d0"),
        }
    }

    pub fn rose_pine() -> Self {
        Self {
            bg: Color::Reset,
            accent: hex_to_color("#ebbcba"), // Rose
            bright: hex_to_color("#e0def4"),
            dim1: hex_to_color("#908caa"),
            dim2: hex_to_color("#6e6a86"),
            dim3: hex_to_color("#312f44"),
            bar: hex_to_color("#ebbcba"),
        }
    }

    pub fn get_by_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "gruvbox" => Self::gruvbox_dark(),
            "tokyo-night" => Self::tokyo_night(),
            "nord" => Self::nord(),
            "rose-pine" => Self::rose_pine(),
            "catppuccin-mocha" | _ => Self::catppuccin_mocha(),
        }
    }
}

pub fn hex_to_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return Color::Reset; // fallback fallback
    }
    
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
    
    Color::Rgb(r, g, b)
}
