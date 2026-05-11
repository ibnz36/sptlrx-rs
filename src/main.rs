mod config;
mod fetcher;
mod lyrics;
mod mpris;
mod raw;
mod theme;
mod ticker;
mod ui;
mod color_extractor;

use tokio::sync::mpsc;

use lyrics::LrcLine;

// ── Tipos compartidos ─────────────────────────────────────────────────────────

/// Información de la canción actual, extraída de los metadatos MPRIS.
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
    pub art_url: Option<String>,
}

/// Eventos que fluyen por el canal mpsc desde los actores hacia la UI.
#[derive(Debug)]
pub enum AppEvent {
    /// Nueva posición en microsegundos, enviada por `mpris::run` cada 250 ms.
    Position(i64),
    /// La canción cambió; incluye título, artista y duración.
    TrackChanged(TrackInfo),
    /// Color dominante de la portada
    DominantColor(ratatui::style::Color),
    /// Letras parseadas listas para mostrar.
    Lyrics(Vec<LrcLine>),
    /// Estado de reproducción: true = reproduciendo, false = en pausa.
    Playing(bool),
    /// El usuario buscó (seek/rewind/forward). Posición en microsegundos.
    /// Viene de la señal DBus `Seeked`, siempre es confiable.
    Seeked(i64),
    /// Pulso del reloj interno cada 50 ms para interpolar la posición.
    Tick,
    /// Señal de cierre (enviada desde la UI al detectar q/Ctrl+C).
    Quit,
}

// ── Ensamblador ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Canal principal: actores → UI
    let (tx, rx) = mpsc::channel::<AppEvent>(128);

    // Canal dedicado: mpris → fetcher (para disparar descargas de letras)
    let (fetch_tx, fetch_rx) = mpsc::channel::<TrackInfo>(16);

    // Actor MPRIS: lee Position y Metadata de Spotify via DBus.
    tokio::spawn(mpris::run(tx.clone(), fetch_tx));

    // Actor Ticker: dispara AppEvent::Tick cada 50 ms.
    tokio::spawn(ticker::run(tx.clone()));

    // Actor Fetcher: descarga letras de LRCLIB al cambiar de canción.
    tokio::spawn(fetcher::run(fetch_rx, tx.clone()));

    let mut is_raw = false;
    for arg in std::env::args() {
        if arg == "--raw" {
            is_raw = true;
        }
    }

    if is_raw {
        raw::run(rx).await?;
    } else {
        // Cargar configuración (temas, etc) solo si usamos TUI
        let config = config::Config::load();
        let theme = config.get_theme();

        // Loop de UI: bloquea el hilo principal hasta que el usuario salga.
        ui::run(rx, theme).await?;
    }

    Ok(())
}
