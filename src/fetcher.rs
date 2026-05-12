use std::time::Duration;

use serde::Deserialize;
use tokio::sync::mpsc::{Receiver, Sender};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::{AppEvent, TrackInfo, lyrics::parse_lrc};

// ── Respuesta de LRCLIB ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LrcLibResult {
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

// ── Limpieza de strings ───────────────────────────────────────────────────────

/// Patrones de ruido dentro de paréntesis/corchetes.
const NOISE_WORDS: [&str; 10] = [
    "remaster",
    "deluxe",
    "bonus",
    "explicit",
    "live",
    "anniversary",
    "edition",
    "expanded",
    "stereo",
    "mono",
];

/// Limpia títulos de Spotify: elimina "(Remastered 2011)", "- Live", etc.
fn clean_title(title: &str) -> String {
    let mut s = title.to_string();

    // Eliminar contenido entre paréntesis/corchetes que sea ruido
    for (open, close) in [('(', ')'), ('[', ']')] {
        loop {
            let Some(start) = s.find(open) else { break };
            let Some(rel_end) = s[start..].find(close) else {
                break;
            };
            let end = start + rel_end + 1;
            let inner = s[start + 1..end - 1].to_lowercase();
            if NOISE_WORDS.iter().any(|w| inner.contains(w)) {
                s = format!("{}{}", s[..start].trim_end(), &s[end..]);
            } else {
                break;
            }
        }
    }

    // Eliminar sufijos con guión: " - Remastered", " - Live Version", etc.
    let lower = s.to_lowercase();
    for pattern in ["- remaster", "- live", "- deluxe", "- bonus", "- stereo"] {
        if let Some(idx) = lower.find(pattern) {
            s = s[..idx].trim_end().to_string();
            break;
        }
    }

    s.trim().to_string()
}

/// Extrae el artista principal (el primero si hay varios separados por ", ").
fn primary_artist(artist: &str) -> &str {
    artist.split(", ").next().unwrap_or(artist)
}

// ── Fetch de letras ───────────────────────────────────────────────────────────

async fn fetch_from_lrclib(client: &reqwest::Client, track: &TrackInfo) -> Option<String> {
    let title = clean_title(&track.title);
    let artist = primary_artist(&track.artist);
    let duration_secs = (track.duration_ms / 1000).to_string();

    // Intento 1: endpoint directo con duración (más preciso)
    let resp = client
        .get("https://lrclib.net/api/get")
        .query(&[
            ("artist_name", artist),
            ("track_name", title.as_str()),
            ("duration", duration_secs.as_str()),
        ])
        .send()
        .await
        .ok()?;

    if resp.status().is_success() {
        if let Ok(data) = resp.json::<LrcLibResult>().await {
            if data.synced_lyrics.is_some() {
                return data.synced_lyrics;
            }
        }
    }

    // Intento 2: búsqueda sin duración (más flexible)
    let resp = client
        .get("https://lrclib.net/api/search")
        .query(&[("artist_name", artist), ("track_name", title.as_str())])
        .send()
        .await
        .ok()?;

    if resp.status().is_success() {
        if let Ok(results) = resp.json::<Vec<LrcLibResult>>().await {
            return results.into_iter().find_map(|r| r.synced_lyrics);
        }
    }

    None
}

async fn get_lyrics(client: &reqwest::Client, track: &TrackInfo) -> Option<String> {
    let cache_path = get_cache_path(track);

    // 1. Intentar leer de la caché
    if let Some(path) = &cache_path {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if !content.trim().is_empty() {
                return Some(content);
            }
        }
    }

    // 2. Fetch de LRCLIB
    let lyrics_opt = fetch_from_lrclib(client, track).await;

    // 3. Guardar en la caché
    if let Some(lyrics) = &lyrics_opt {
        if let Some(path) = &cache_path {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = tokio::fs::write(path, lyrics).await;
        }
    }

    lyrics_opt
}

fn get_cache_path(track: &TrackInfo) -> Option<PathBuf> {
    let mut cache_dir = dirs::cache_dir()?;
    cache_dir.push("sptlrx-rs");

    let mut hasher = DefaultHasher::new();
    track.artist.hash(&mut hasher);
    track.title.hash(&mut hasher);
    let hash = hasher.finish();

    cache_dir.push(format!("{:x}.lrc", hash));
    Some(cache_dir)
}

// ── Actor principal ───────────────────────────────────────────────────────────

/// Recibe `TrackInfo` del actor MPRIS, descarga letras de LRCLIB y envía
/// `AppEvent::Lyrics` al canal de la UI.
/// Soporta cancelación: si llega un nuevo track durante la descarga,
/// cancela la petición en curso y empieza la nueva.
pub async fn run(mut rx: Receiver<TrackInfo>, tx: Sender<AppEvent>) {
    let client = reqwest::Client::builder()
        .user_agent("sptlrx-rs/0.1 (https://github.com/user/sptlrx-rs)")
        .timeout(Duration::from_secs(8))
        .build()
        .expect("Could not create HTTP client");

    let mut pending: Option<TrackInfo> = None;

    loop {
        // Obtener el siguiente track (nuevo o pendiente de cancelación)
        let track = if let Some(t) = pending.take() {
            t
        } else {
            match rx.recv().await {
                Some(t) => t,
                _ => break, // Canal cerrado
            }
        };

        // Drenar tracks más recientes (el usuario saltó canciones rápido)
        let mut current = track;
        while let Ok(newer) = rx.try_recv() {
            current = newer;
        }

        // Fetch con cancelación: si llega un nuevo track, aborta la descarga
        let fetch = get_lyrics(&client, &current);
        tokio::select! {
            result = fetch => {
                let lines = match result {
                    Some(lrc_text) => parse_lrc(&lrc_text),
                    _ => Vec::new(),
                };
                let _ = tx.send(AppEvent::Lyrics(lines)).await;
            }
            newer = rx.recv() => {
                // Nuevo track llegó durante la descarga → cancelar y reintentar
                if let Some(t) = newer {
                    pending = Some(t);
                } else {
                    break;
                }
            }
        }
    }
}
