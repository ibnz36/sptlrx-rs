use serde::Deserialize;
use std::fs;

use crate::theme::{hex_to_color, Theme};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub theme: Option<String>,
    pub custom: Option<CustomThemeConfig>,
    pub background: Option<String>, // Opcional para forzar un fondo sólido
}

#[derive(Debug, Deserialize)]
pub struct CustomThemeConfig {
    pub accent: Option<String>,
    pub text: Option<String>,
    pub dim1: Option<String>,
    pub dim2: Option<String>,
    pub dim3: Option<String>,
    pub bar: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        // Buscar en ~/.config/sptlrx-rs/config.toml
        if let Some(mut config_path) = dirs::config_dir() {
            config_path.push("sptlrx-rs");
            config_path.push("config.toml");

            if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str::<Config>(&content) {
                    return config;
                }
            }
        }

        // Default config si no hay archivo o hay error al parsear
        Self {
            theme: Some("catppuccin-mocha".to_string()),
            custom: None,
            background: None,
        }
    }

    pub fn get_theme(&self) -> Theme {
        let mut theme = match self.theme.as_deref() {
            Some("custom") => Theme::catppuccin_mocha(), // Base para override
            Some(name) => Theme::get_by_name(name),
            _ => Theme::catppuccin_mocha(),
        };

        // Sobreescribir con custom si existe y se eligió 'custom'
        if self.theme.as_deref() == Some("custom") {
            if let Some(custom) = &self.custom {
                if let Some(accent) = &custom.accent { theme.accent = hex_to_color(accent); }
                if let Some(text) = &custom.text { theme.bright = hex_to_color(text); }
                if let Some(dim1) = &custom.dim1 { theme.dim1 = hex_to_color(dim1); }
                if let Some(dim2) = &custom.dim2 { theme.dim2 = hex_to_color(dim2); }
                if let Some(dim3) = &custom.dim3 { theme.dim3 = hex_to_color(dim3); }
                if let Some(bar) = &custom.bar { theme.bar = hex_to_color(bar); }
            }
        }

        // Aplicar fondo sólido si el usuario lo pide explícitamente, sino se queda transparente (Reset)
        if let Some(bg_hex) = &self.background {
            theme.bg = hex_to_color(bg_hex);
        }

        theme
    }
}
