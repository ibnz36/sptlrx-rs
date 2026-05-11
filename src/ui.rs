use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Wrap},
    Frame, Terminal,
};
use tokio::sync::mpsc::Receiver;

use crate::{
    lyrics::{find_current_line, LrcLine},
    theme::Theme,
    AppEvent, TrackInfo,
};

// ── Estado de la aplicación ───────────────────────────────────────────────────

struct AppState {
    theme: Theme,
    lyrics: Vec<LrcLine>,
    track_title: String,
    track_artist: String,
    duration_ms: u64,
    /// Base de interpolación
    last_known_pos_ms: u64,
    /// Instante del último re-anclaje
    last_sync_time: Instant,
    current_line: Option<usize>,
    lyrics_loading: bool,
    is_playing: bool,
    /// false = aún no hemos recibido una posición fiable (app recién iniciada)
    initial_sync_done: bool,
}

impl AppState {
    fn new(theme: Theme) -> Self {
        Self {
            theme,
            lyrics: Vec::new(),
            track_title: String::from("Esperando Spotify..."),
            track_artist: String::new(),
            duration_ms: 0,
            last_known_pos_ms: 0,
            last_sync_time: Instant::now(),
            current_line: None,
            lyrics_loading: true,
            is_playing: false,
            initial_sync_done: false,
        }
    }

    /// Posición interpolada:
    /// - Si está reproduciendo: última posición MPRIS + tiempo transcurrido.
    /// - Si está en pausa: devuelve la última posición conocida sin avanzar.
    fn interpolated_pos_ms(&self) -> u64 {
        if self.is_playing {
            self.last_known_pos_ms + self.last_sync_time.elapsed().as_millis() as u64
        } else {
            self.last_known_pos_ms
        }
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Position(pos_us) => {
                // Position solo se usa UNA VEZ al iniciar la app.
                // Después, Seeked es la fuente autoritativa.
                // Razón: Spotify MPRIS devuelve un valor ESTÁTICO que
                // sobreescribe los valores correctos de Seeked.
                if !self.initial_sync_done {
                    let new_pos_ms = (pos_us.max(0) / 1000) as u64;
                    self.initial_sync_done = true;
                    self.last_known_pos_ms = new_pos_ms;
                    self.last_sync_time = Instant::now();
                }
            }
            AppEvent::Playing(playing) => {
                if playing && !self.is_playing {
                    self.last_sync_time = Instant::now();
                }
                self.is_playing = playing;
            }
            AppEvent::Seeked(pos_us) => {
                let pos_ms = (pos_us.max(0) / 1000) as u64;
                self.initial_sync_done = true;
                self.last_known_pos_ms = pos_ms;
                self.last_sync_time = Instant::now();
            }
            AppEvent::Tick => {
                let pos = self.interpolated_pos_ms();
                self.current_line = find_current_line(&self.lyrics, pos);
            }
            AppEvent::TrackChanged(TrackInfo { title, artist, duration_ms }) => {
                self.track_title = title;
                self.track_artist = artist;
                self.duration_ms = duration_ms;
                self.lyrics.clear();
                self.lyrics_loading = true;
                self.current_line = None;
                self.last_known_pos_ms = 0;
                self.last_sync_time = Instant::now();
                // NO resetear initial_sync_done: Seeked llegará con la posición correcta
            }
            AppEvent::Lyrics(lines) => {
                self.lyrics_loading = false;
                self.lyrics = lines;
                self.current_line = None;
            }
            AppEvent::Quit => {}
        }
    }
}

// ── Helpers de formato ────────────────────────────────────────────────────────





// ── Renderizado ───────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.size();

    // Fondo general
    frame.render_widget(
        Block::default().style(Style::default().bg(state.theme.bg)),
        area,
    );

    // Letras en pantalla completa
    render_lyrics(frame, state, area);
}



fn render_lyrics(frame: &mut Frame, state: &AppState, area: Rect) {
    if state.lyrics.is_empty() {
        let msg = if state.lyrics_loading {
            "⏳ Buscando letras..."
        } else {
            "No se encontraron letras sincronizadas"
        };
        let placeholder = Paragraph::new(msg)
            .style(Style::default().fg(state.theme.dim1).bg(state.theme.bg))
            .alignment(Alignment::Center);
        frame.render_widget(placeholder, area);
        return;
    }

    let current = state.current_line.unwrap_or(0);
    let lyric = &state.lyrics[current];

    let available_height = area.height as usize;
    let top_padding = available_height.saturating_sub(1) / 2;

    let mut lines: Vec<Line> = Vec::new();

    // Padding superior para centrar verticalmente
    for _ in 0..top_padding {
        lines.push(Line::from(""));
    }

    // Solo la línea actual, sin indicador, centrada
    lines.push(Line::from(vec![
        Span::styled(
            lyric.text.clone(),
            Style::default()
                .fg(state.theme.bright)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let lyrics_widget = Paragraph::new(Text::from(lines))
        .alignment(Alignment::Center)
        .style(Style::default().bg(state.theme.bg))
        .wrap(Wrap { trim: false });

    frame.render_widget(lyrics_widget, area);
}


// ── Setup / teardown del terminal ─────────────────────────────────────────────

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

// ── Loop principal ────────────────────────────────────────────────────────────

/// Inicializa el terminal, arranca el loop de renderizado y maneja el cleanup.
pub async fn run(mut rx: Receiver<AppEvent>, theme: Theme) -> anyhow::Result<()> {
    let mut terminal = setup_terminal()?;
    let mut state = AppState::new(theme);
    let frame_duration = Duration::from_millis(16); // ~60 FPS

    loop {
        // ── Dibujar ───────────────────────────────────────────────────────
        terminal.draw(|f| render(f, &state))?;

        // ── Drenar canal mpsc sin bloquear ────────────────────────────────
        while let Ok(event) = rx.try_recv() {
            state.handle_event(event);
        }

        // ── Eventos de teclado (non-blocking) ─────────────────────────────
        if event::poll(Duration::from_millis(0))? {
            if let Event::Key(key) = event::read()? {
                let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL));
                if quit {
                    break;
                }
            }
        }

        tokio::time::sleep(frame_duration).await;
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}
