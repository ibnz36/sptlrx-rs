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
    let connection = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[mpris] No se pudo conectar a DBus: {e}");
            return;
        }
    };

    let player = match PlayerProxy::new(&connection).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[mpris] No se pudo crear el proxy de Spotify: {e}");
            return;
        }
    };

    // Stream de señales Seeked (se dispara al hacer seek/rewind/forward)
    let mut seeked_stream = match player.receive_seeked().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[mpris] No se pudo escuchar señal Seeked: {e}");
            return;
        }
    };

    let mut last_track_id = String::new();
    let mut interval = time::interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            // ── Rama 1: Polling periódico (100ms) ─────────────────────────
            _ = interval.tick() => {
                // 1. Metadata: detectar cambio de canción PRIMERO
                let mut track_just_changed = false;
                if let Ok(meta) = player.metadata().await {
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

                            let info = TrackInfo { title, artist, duration_ms };
                            let _ = tx.send(AppEvent::TrackChanged(info.clone())).await;
                            let _ = fetch_tx.send(info).await;
                        }
                    }
                }

                // 2. PlaybackStatus
                if let Ok(status) = player.playback_status().await {
                    let _ = tx.send(AppEvent::Playing(status == "Playing")).await;
                }

                // 3. Position (skip si hubo cambio de canción)
                if !track_just_changed {
                    if let Ok(pos_us) = player.position().await {
                        if tx.send(AppEvent::Position(pos_us)).await.is_err() {
                            break;
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
}
