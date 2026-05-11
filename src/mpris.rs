use std::collections::HashMap;
use std::ops::Deref;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc::Sender;
use tokio::time;
use zbus::zvariant::OwnedValue;
use zbus::{dbus_proxy, Connection};

use crate::{AppEvent, TrackInfo};

#[dbus_proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_service = "org.mpris.MediaPlayer2.spotify",
    default_path = "/org/mpris/MediaPlayer2"
)]
trait Player {
    #[dbus_proxy(property)]
    fn position(&self) -> zbus::Result<i64>;

    #[dbus_proxy(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;

    #[dbus_proxy(property)]
    fn playback_status(&self) -> zbus::Result<String>;

    /// Señal emitida por Spotify al hacer seek (adelantar/regresar).
    /// El argumento es la nueva posición en microsegundos.
    #[dbus_proxy(signal)]
    fn seeked(&self, position: i64) -> zbus::Result<()>;

    // ── Métodos de control interactivo ──
    #[dbus_proxy(name = "PlayPause")]
    fn play_pause_track(&self) -> zbus::Result<()>;
    
    #[dbus_proxy(name = "Next")]
    fn next_track(&self) -> zbus::Result<()>;
    
    #[dbus_proxy(name = "Previous")]
    fn previous_track(&self) -> zbus::Result<()>;
    
    #[dbus_proxy(name = "Seek")]
    fn seek_track(&self, offset: i64) -> zbus::Result<()>;
}

pub async fn toggle_play_pause() {
    tokio::spawn(async {
        if let Ok(conn) = Connection::session().await {
            if let Ok(player) = PlayerProxy::new(&conn).await {
                let _ = player.play_pause_track().await;
            }
        }
    });
}

pub async fn next_track() {
    tokio::spawn(async {
        if let Ok(conn) = Connection::session().await {
            if let Ok(player) = PlayerProxy::new(&conn).await {
                let _ = player.next_track().await;
            }
        }
    });
}

pub async fn previous_track() {
    tokio::spawn(async {
        if let Ok(conn) = Connection::session().await {
            if let Ok(player) = PlayerProxy::new(&conn).await {
                let _ = player.previous_track().await;
            }
        }
    });
}

pub async fn seek_relative(offset_us: i64) {
    tokio::spawn(async move {
        if let Ok(conn) = Connection::session().await {
            if let Ok(player) = PlayerProxy::new(&conn).await {
                let _ = player.seek_track(offset_us).await;
            }
        }
    });
}

// ── Helpers para extraer tipos de OwnedValue ─────────────────────────────────

fn get_str(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v = metadata.get(key)?;
    match v.deref() {
        zbus::zvariant::Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn get_str_array(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v = metadata.get(key)?;
    match v.deref() {
        zbus::zvariant::Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|item| match item {
                    zbus::zvariant::Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect();
            if parts.is_empty() { None } else { Some(parts.join(", ")) }
        }
        _ => None,
    }
}

fn get_i64(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<i64> {
    let v = metadata.get(key)?;
    match v.deref() {
        zbus::zvariant::Value::I64(n) => Some(*n),
        zbus::zvariant::Value::U64(n) => Some(*n as i64),
        _ => None,
    }
}

fn get_track_id(metadata: &HashMap<String, OwnedValue>) -> Option<String> {
    let v = metadata.get("mpris:trackid")?;
    match v.deref() {
        zbus::zvariant::Value::ObjectPath(p) => Some(p.to_string()),
        zbus::zvariant::Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

// ── Actor principal ───────────────────────────────────────────────────────────

/// Bucle asíncrono que:
/// 1. Pollea Metadata → PlaybackStatus → Position cada 100ms.
/// 2. Escucha la señal DBus `Seeked` para detectar seeks al instante.
pub async fn run(tx: Sender<AppEvent>, fetch_tx: Sender<TrackInfo>) {
    loop {
        let connection = match Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[mpris] No se pudo conectar a DBus: {e}. Reintentando en 2s...");
                time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        let player = match PlayerProxy::new(&connection).await {
            Ok(p) => p,
            Err(_) => {
                // Spotify no está abierto, enviamos evento de "Esperando"
                let _ = tx.send(AppEvent::TrackChanged(TrackInfo {
                    title: "Waiting for Spotify...".to_string(),
                    artist: String::new(),
                    duration_ms: 0,
                    art_url: None,
                })).await;
                time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        // Stream de señales Seeked (se dispara al hacer seek/rewind/forward)
        let mut seeked_stream = match player.receive_seeked().await {
            Ok(s) => s,
            Err(_) => {
                time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        let mut last_track_id = String::new();
        let mut interval = time::interval(Duration::from_millis(100));
        let mut error_count = 0;

        loop {
            tokio::select! {
                // ── Rama 1: Polling periódico (100ms) ─────────────────────────
                _ = interval.tick() => {
                    let mut track_just_changed = false;
                    
                    // Intentamos obtener metadata
                    match player.metadata().await {
                        Ok(meta) => {
                            error_count = 0;
                            if let Some(track_id) = get_track_id(&meta) {
                                if track_id != last_track_id {
                                    last_track_id = track_id;
                                    track_just_changed = true;

                                    let title = get_str(&meta, "xesam:title")
                                        .unwrap_or_else(|| "Desconocido".to_string());
                                    let artist = get_str_array(&meta, "xesam:artist")
                                        .unwrap_or_else(|| "Artista desconocido".to_string());
                                    let duration_ms = get_i64(&meta, "mpris:length")
                                        .map(|us| (us / 1000) as u64)
                                        .unwrap_or(0);
                                    let art_url = get_str(&meta, "mpris:artUrl");

                                    let info = TrackInfo { title, artist, duration_ms, art_url: art_url.clone() };
                                    let _ = tx.send(AppEvent::TrackChanged(info.clone())).await;
                                    let _ = fetch_tx.send(info).await;
                                    
                                    if let Some(url) = art_url {
                                        let tx_clone = tx.clone();
                                        tokio::spawn(async move {
                                            if let Some((color, img)) = crate::color_extractor::get_dominant_color(&url).await {
                                                let _ = tx_clone.send(AppEvent::ArtProcessed(color, img)).await;
                                            }
                                        });
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            error_count += 1;
                        }
                    }

                    if error_count > 10 {
                        // Si hay muchos errores seguidos, probablemente Spotify se cerró.
                        // Salimos del loop interno para volver al loop de reconexión.
                        break;
                    }

                    // 2. PlaybackStatus
                    if let Ok(status) = player.playback_status().await {
                        let _ = tx.send(AppEvent::Playing(status == "Playing")).await;
                    }

                    // 3. Position (skip si hubo cambio de canción)
                    if !track_just_changed {
                        if let Ok(pos_us) = player.position().await {
                            if tx.send(AppEvent::Position(pos_us)).await.is_err() {
                                return; // El canal se cerró, la app está cerrando
                            }
                        }
                    }
                }

                // ── Rama 2: Señal Seeked (instantánea) ────────────────────────
                Some(signal) = seeked_stream.next() => {
                    if let Ok(args) = signal.args() {
                        let _ = tx.send(AppEvent::Seeked(args.position)).await;
                    }
                }
            }
        }
        
        // Esperamos un poco antes de intentar reconectar después de un fallo
        time::sleep(Duration::from_secs(1)).await;
    }
}
