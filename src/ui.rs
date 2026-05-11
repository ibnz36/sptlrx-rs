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
    style::{Color, Modifier, Style},
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
    base_bright: Color,
    target_bright: Option<Color>,
    album_art: Option<image::RgbImage>,
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
    /// Desplazamiento visual para interpolación de color
    visual_offset: f64,
    /// Tiempo de animación para efectos continuos
    animation_time: f64,
    /// Detector de tramo instrumental
    is_instrumental: bool,
}

impl AppState {
    fn new(theme: Theme) -> Self {
        Self {
            base_bright: theme.bright,
            target_bright: None,
            album_art: None,
            theme,
            lyrics: Vec::new(),
            track_title: String::from("Waiting for Spotify..."),
            track_artist: String::new(),
            duration_ms: 0,
            last_known_pos_ms: 0,
            last_sync_time: Instant::now(),
            current_line: None,
            lyrics_loading: true,
            is_playing: false,
            initial_sync_done: false,
            visual_offset: 0.0,
            animation_time: 0.0,
            is_instrumental: false,
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
            AppEvent::TrackChanged(TrackInfo { title, artist, duration_ms, .. }) => {
                self.track_title = title;
                self.track_artist = artist;
                self.duration_ms = duration_ms;
                self.lyrics.clear();
                self.lyrics_loading = true;
                self.current_line = None;
                self.last_known_pos_ms = 0;
                self.last_sync_time = Instant::now();
                self.target_bright = None;
                self.album_art = None;
                // NO resetear initial_sync_done: Seeked llegará con la posición correcta
            }
            AppEvent::ArtProcessed(color, img) => {
                self.target_bright = Some(color);
                self.album_art = Some(img);
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

// ── Format helpers ────────────────────────────────────────────────────────

fn get_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255), // Fallback
    }
}

fn lerp_color(c1: Color, c2: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (r1, g1, b1) = get_rgb(c1);
    let (r2, g2, b2) = get_rgb(c2);
    let r = (r1 as f64 + (r2 as f64 - r1 as f64) * t) as u8;
    let g = (g1 as f64 + (g2 as f64 - g1 as f64) * t) as u8;
    let b = (b1 as f64 + (b2 as f64 - b1 as f64) * t) as u8;
    Color::Rgb(r, g, b)
}

/// Devuelve un `Style` con degradado según la distancia a la línea activa.
fn dim_style(distance: f64, theme: &Theme) -> Style {
    let color = if distance < 1.0 {
        lerp_color(theme.bright, theme.dim1, distance)
    } else if distance < 2.0 {
        lerp_color(theme.dim1, theme.dim2, distance - 1.0)
    } else if distance < 3.0 {
        lerp_color(theme.dim2, theme.dim3, distance - 2.0)
    } else if distance < 5.0 {
        // Suave desvanecimiento hacia el color de fondo absoluto
        lerp_color(theme.dim3, theme.bg, (distance - 3.0) / 2.0)
    } else {
        theme.bg
    };
    Style::default().fg(color)
}

// ── Renderizado ───────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, state: &AppState) {
    let mut area = frame.size();

    // Fondo general
    frame.render_widget(
        Block::default().style(Style::default().bg(state.theme.bg)),
        area,
    );

    let progress_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    
    // Reducir área vertical para dejar espacio a la barra de progreso
    area.height = area.height.saturating_sub(1);

    let mut lyrics_area = area;
    let art_width = 30; // 24 block_width + 6 padding

    // Si hay portada y espacio suficiente, desplazamos las letras a la derecha
    if state.album_art.is_some() && area.width > art_width + 20 {
        lyrics_area.x += art_width + 4; // Añadir margen izquierdo extra
        lyrics_area.width = lyrics_area.width.saturating_sub(art_width + 4);
    } else {
        // Margen por defecto si está centrado
        lyrics_area.x += 2;
        lyrics_area.width = lyrics_area.width.saturating_sub(4);
    }

    // Letras en pantalla (reducida o completa)
    render_lyrics(frame, state, lyrics_area);
    
    // Portada en la esquina superior izquierda
    render_album_art(frame, state, area);
    
    // Barra de progreso
    render_progress_bar(frame, state, progress_area);
}



fn render_lyrics(frame: &mut Frame, state: &AppState, area: Rect) {
    if state.lyrics.is_empty() {
        let msg = if state.lyrics_loading {
            "⏳ Fetching lyrics..."
        } else {
            "No synced lyrics found"
        };
        let placeholder = Paragraph::new(msg)
            .style(Style::default().fg(state.theme.dim1).bg(state.theme.bg))
            .alignment(Alignment::Center);
        frame.render_widget(placeholder, area);
        return;
    }

    let current = state.current_line.unwrap_or(0);
    // Usar todo el espacio vertical disponible (mitad para arriba, mitad para abajo)
    let context = (area.height as usize).saturating_div(2).saturating_sub(1);

    let start = current.saturating_sub(context / 2);
    let end = (current + context / 2 + 1).min(state.lyrics.len());

    let mut visible_lines = (end - start) * 2;
    if state.is_instrumental && current >= start && current < end {
        visible_lines += 2;
    }
    
    let available_height = area.height as usize;
    let top_padding = available_height.saturating_sub(visible_lines) / 2;

    let mut lines: Vec<Line> = Vec::new();

    // Padding superior para centrar verticalmente
    for _ in 0..top_padding {
        lines.push(Line::from(""));
    }

    for i in start..end {
        let lyric = &state.lyrics[i];
        let distance = (i as f64 - state.visual_offset).abs();

        let line_style = dim_style(distance, &state.theme);
        
        // Línea más cercana al centro visual: negrita + Glow Sweep
        if distance < 0.25 {
            let mut spans = Vec::with_capacity(lyric.text.chars().count());
            let len = lyric.text.chars().count() as f64;
            
            // El centro del brillo viaja de izquierda a derecha continuamente
            let sweep_center = (state.animation_time * 15.0).rem_euclid(len + 30.0) - 15.0;
            
            for (char_idx, c) in lyric.text.chars().enumerate() {
                let char_dist = (char_idx as f64 - sweep_center).abs();
                let glow_intensity = (1.0 - char_dist / 4.0).clamp(0.0, 1.0);
                
                let char_color = if glow_intensity > 0.0 {
                    // Mezclamos el color base (camaleón) con blanco puro para dar sensación de luz
                    lerp_color(state.theme.bright, Color::Rgb(255, 255, 255), glow_intensity * 0.8)
                } else {
                    state.theme.bright
                };
                
                spans.push(Span::styled(c.to_string(), Style::default().fg(char_color).add_modifier(Modifier::BOLD)));
            }
            
            lines.push(Line::from(spans));
            lines.push(Line::from("")); // Doble espaciado
            
        } else {
            // Líneas de contexto: Desvanecimiento puro estilo Apple Music (sin glitch)
            lines.push(Line::from(Span::styled(lyric.text.clone(), line_style)));
            lines.push(Line::from("")); // Doble espaciado
        }
        
        // Insertar animación de instrumental en el hueco
        if state.is_instrumental && i == current {
            let wave_chars = ['〰', '🎵', '〰', '〰', '🎶', '〰'];
            let offset = (state.animation_time * 3.0) as usize;
            let mut wave_str = String::new();
            for j in 0..15 {
                wave_str.push(wave_chars[(j + offset) % wave_chars.len()]);
                wave_str.push(' ');
            }
            
            let instr_dist = (i as f64 + 0.5 - state.visual_offset).abs();
            let instr_style = dim_style(instr_dist, &state.theme);
            let final_style = if instr_dist < 0.25 {
                instr_style.add_modifier(Modifier::BOLD)
            } else {
                instr_style
            };
            lines.push(Line::from(Span::styled(wave_str, final_style)));
            lines.push(Line::from("")); // Doble espaciado
        }
    }

    let alignment = if state.album_art.is_some() && area.width > 50 {
        Alignment::Left
    } else {
        Alignment::Center
    };

    let lyrics_widget = Paragraph::new(Text::from(lines))
        .alignment(alignment)
        .style(Style::default().bg(state.theme.bg))
        .wrap(Wrap { trim: false });

    frame.render_widget(lyrics_widget, area);
}

fn render_album_art(frame: &mut Frame, state: &AppState, area: Rect) {
    if let Some(img) = &state.album_art {
        let block_width = 24; // 24 columnas
        let block_height = 12; // 12 filas (24 pixeles de alto)
        
        // Si no hay suficiente espacio para la portada, no la dibujamos
        if area.width < block_width + 6 || area.height < block_height + 4 {
            return;
        }

        let art_area = Rect {
            x: area.x + 3,
            y: area.y + 2,
            width: block_width,
            height: block_height,
        };

        // Redimensionar para encajar en los caracteres (FilterType::Triangle para velocidad/calidad)
        let scaled = image::imageops::resize(img, block_width as u32, (block_height * 2) as u32, image::imageops::FilterType::Triangle);
        
        let mut lines = Vec::new();
        for y in (0..scaled.height()).step_by(2) {
            let mut spans = Vec::new();
            for x in 0..scaled.width() {
                let top = scaled.get_pixel(x, y);
                let bottom = if y + 1 < scaled.height() { scaled.get_pixel(x, y + 1) } else { top };
                
                spans.push(Span::styled(
                    "▀",
                    Style::default()
                        .fg(Color::Rgb(top[0], top[1], top[2]))
                        .bg(Color::Rgb(bottom[0], bottom[1], bottom[2]))
                ));
            }
            lines.push(Line::from(spans));
        }

        frame.render_widget(Paragraph::new(lines), art_area);
    }
}

fn render_progress_bar(frame: &mut Frame, state: &AppState, area: Rect) {
    if state.duration_ms == 0 {
        return;
    }

    let progress = (state.interpolated_pos_ms() as f64 / state.duration_ms as f64).clamp(0.0, 1.0);
    
    let width = area.width as usize;
    if width == 0 { return; }

    let filled_width = (width as f64 * progress).round() as usize;
    let mut line_spans = Vec::new();
    
    // Braille progress
    if filled_width > 0 {
        line_spans.push(Span::styled(
            "⣿".repeat(filled_width),
            Style::default().fg(state.theme.bright)
        ));
    }
    
    let empty_width = width.saturating_sub(filled_width);
    if empty_width > 0 {
        line_spans.push(Span::styled(
            "⣀".repeat(empty_width),
            Style::default().fg(state.theme.dim3)
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(line_spans)), area);
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

        // Detección de tramo instrumental
        let mut target_offset = state.current_line.unwrap_or(0) as f64;
        let pos = state.interpolated_pos_ms();
        state.is_instrumental = false;
        
        if let Some(curr) = state.current_line {
            if let (Some(curr_lyric), Some(next_lyric)) = (state.lyrics.get(curr), state.lyrics.get(curr + 1)) {
                // Si pasaron 5s y faltan >10s
                if pos > curr_lyric.timestamp_ms + 5000 && next_lyric.timestamp_ms > pos + 10000 {
                    state.is_instrumental = true;
                    target_offset += 0.5; // Apuntar visualmente al medio
                }
            }
        }

        // Interpolación visual a 60fps
        state.visual_offset += (target_offset - state.visual_offset) * 0.15;
        state.animation_time += 0.05;

        // Interpolación de color camaleón a 60fps
        if let Some(target) = state.target_bright {
            state.theme.bright = lerp_color(state.theme.bright, target, 0.05);
        } else {
            state.theme.bright = lerp_color(state.theme.bright, state.base_bright, 0.05);
        }

        tokio::time::sleep(frame_duration).await;
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}
