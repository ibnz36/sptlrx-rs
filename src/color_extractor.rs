use ratatui::style::Color;
use std::collections::HashMap;

pub async fn get_dominant_color(url: &str) -> Option<Color> {
    let bytes = if url.starts_with("http") {
        reqwest::get(url).await.ok()?.bytes().await.ok()?
    } else if url.starts_with("file://") {
        // Decodificar la URL por si tiene espacios (%20)
        let path = url.trim_start_matches("file://");
        let decoded = urlencoding::decode(path).ok()?;
        tokio::fs::read(decoded.as_ref()).await.ok()?.into()
    } else {
        return None;
    };

    // Decodificar y reducir imagen en otro hilo para no bloquear
    let img = tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&bytes).ok()?;
        // 64x64 es suficientemente rápido y da una buena media
        Some(img.thumbnail_exact(64, 64).to_rgb8())
    })
    .await
    .ok()??;

    // Agrupar colores
    let mut buckets: HashMap<(u8, u8, u8), usize> = HashMap::new();
    for pixel in img.pixels() {
        // Cuantizar el color a paleta más pequeña (ignorar los 4 bits menos significativos)
        let r = pixel[0] & 0xF0;
        let g = pixel[1] & 0xF0;
        let b = pixel[2] & 0xF0;
        *buckets.entry((r, g, b)).or_insert(0) += 1;
    }

    // Buscar el color dominante, penalizando tonos grises oscuros
    let mut max_score = 0;
    let mut dominant = (255, 255, 255);

    for (color, count) in buckets {
        let (r, g, b) = color;
        let max_val = r.max(g).max(b);
        let min_val = r.min(g).min(b);
        let saturation = max_val.saturating_sub(min_val);
        
        // Multiplicador de "vistosidad": preferimos colores saturados
        let score = count * (1 + saturation as usize);

        if score > max_score {
            max_score = score;
            dominant = color;
        }
    }

    // Aumentar el brillo mínimo para que el texto sea legible (es el texto brillante)
    let (mut r, mut g, mut b) = dominant;
    let max_c = r.max(g).max(b);
    if max_c < 180 {
        let scale = 180.0 / (max_c as f32).max(1.0);
        r = ((r as f32) * scale).min(255.0) as u8;
        g = ((g as f32) * scale).min(255.0) as u8;
        b = ((b as f32) * scale).min(255.0) as u8;
    }

    // Suavizar los colores para asegurar que se mezcle bien
    // Al ser cuantizado, le sumamos 8 a cada canal para ponerlo en el "medio" del bucket
    r = r.saturating_add(8);
    g = g.saturating_add(8);
    b = b.saturating_add(8);

    Some(Color::Rgb(r, g, b))
}
