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
                if playing != self.is_playing {
                    // Al cambiar estado, re-anclamos la posición para evitar desincronización
                    if !playing {
                        // Si pausamos, capturamos el milisegundo exacto de la interpolación
                        self.last_known_pos_ms = self.interpolated_pos_ms();
                    }
                    self.last_sync_time = Instant::now();
                    self.is_playing = playing;
                }
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

    // Matemática responsiva inteligente (Smart Scaling):
    // El ancho intenta ser 1/3 de la pantalla, pero se ajusta si el alto es escaso.
    let max_art_width = (area.width / 3).clamp(10, 30);
    let max_art_height = (area.height.saturating_sub(8) as u16).max(5); // Reservar espacio para info/botones
    
    let art_width = max_art_width.min(max_art_height * 2);
    let art_height = art_width / 2;

    // Decidimos si hay espacio para el layout de "Reproductor Completo"
    let show_full_ui = state.album_art.is_some() 
        && area.width > art_width + 20 
        && area.height > art_height + 6;

    if state.lyrics.is_empty() && !state.lyrics_loading && state.album_art.is_some() {
        // ── CASO: PORTADA CENTRADA (Sin letras encontradas) ──
        // Matemática responsiva para el centro:
        let max_w = (area.width / 2).clamp(20, 50).min(area.width.saturating_sub(4));
        let max_h = (area.height.saturating_sub(8) as u16).max(5); 
        
        let centered_width = max_w.min(max_h * 2);
        let centered_height = centered_width / 2;
        
        let x = (area.width.saturating_sub(centered_width)) / 2;
        let y = (area.height.saturating_sub(centered_height + 6)) / 2;

        let art_rect = Rect { x, y, width: centered_width, height: centered_height };
        let info_rect = Rect { x: 0, y: y + centered_height + 1, width: area.width, height: 7 };

        let safe_art = art_rect.intersection(area);
        let safe_info = info_rect.intersection(area);

        if safe_art.height > 0 { render_album_art_rect(frame, state, safe_art); }
        if safe_info.height > 0 { render_track_info_rect(frame, state, safe_info, Alignment::Center); }
        
    } else if show_full_ui {
        // ── CASO: SIDE-BY-SIDE (Reproductor completo con letras) ──
        // Restaurar margen izquierdo pegado (al gusto del usuario)
        lyrics_area.x += art_width + 8;
        lyrics_area.width = lyrics_area.width.saturating_sub(art_width + 12);

        let total_block_height = art_height + 7; 
        // Bajamos un poco más el offset vertical (+2) para que no se sienta "tan arriba"
        let y_offset = (area.y + (area.height.saturating_sub(total_block_height)) / 2).saturating_add(2);

        let art_rect = Rect { x: area.x + 3, y: y_offset, width: art_width, height: art_height };
        let info_rect = Rect { x: area.x + 3, y: y_offset + art_height + 1, width: art_width, height: 6 };

        let safe_art = art_rect.intersection(area);
        let safe_info = info_rect.intersection(area);

        render_lyrics(frame, state, lyrics_area);
        if safe_art.height > 0 { render_album_art_rect(frame, state, safe_art); }
        if safe_info.height > 0 { render_track_info_rect(frame, state, safe_info, Alignment::Center); }
    } else {
        // ── CASO: SOLO LETRAS (Minimalista) ──
        lyrics_area.x = lyrics_area.x.saturating_add(2);
        lyrics_area.width = lyrics_area.width.saturating_sub(4);
        render_lyrics(frame, state, lyrics_area);
    }
    
    // Barra de progreso siempre visible
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
    
    // MODO FOCO: 2 arriba, 1 actual, 2 abajo
    let context_above = 2;
    let context_below = 2;
    
    let start = current.saturating_sub(context_above);
    let end = (current + context_below + 1).min(state.lyrics.len());

    // Calculamos la altura necesaria para 5 líneas con doble espaciado
    let total_lines_to_render = (end - start) * 2;
    let available_height = area.height as usize;
    let top_padding = available_height.saturating_sub(total_lines_to_render) / 2;

    let mut lines: Vec<Line> = Vec::new();

    // Padding superior para centrar el bloque de 5 líneas
    for _ in 0..top_padding {
        lines.push(Line::from(""));
    }

    for i in start..end {
        let lyric = &state.lyrics[i];
        let distance = (i as f64 - state.visual_offset).abs();

        let mut line_style = dim_style(distance, &state.theme);
        
        // Estética: Cursiva para las líneas de fondo (contexto)
        if distance >= 0.25 {
            line_style = line_style.add_modifier(Modifier::ITALIC);
        }

        // Línea más cercana al centro visual: negrita + Glow Sweep
        if distance < 0.25 {
            let mut spans = Vec::with_capacity(lyric.text.chars().count());
            let len = lyric.text.chars().count() as f64;
            
            // El centro del brillo viaja de izquierda a derecha continuamente
            let sweep_center = (state.animation_time * 12.0).rem_euclid(len + 30.0) - 15.0;
            
            for (char_idx, c) in lyric.text.chars().enumerate() {
                let char_dist = (char_idx as f64 - sweep_center).abs();
                let glow_intensity = (1.0 - char_dist / 5.0).clamp(0.0, 1.0);
                
                let char_color = if glow_intensity > 0.0 {
                    lerp_color(state.theme.bright, Color::Rgb(255, 255, 255), glow_intensity)
                } else {
                    state.theme.bright
                };
                
                spans.push(Span::styled(c.to_string(), Style::default().fg(char_color).add_modifier(Modifier::BOLD)));
            }
            
            lines.push(Line::from(spans));
            
        } else {
            // Líneas de contexto: Desvanecimiento puro estilo Apple Music
            lines.push(Line::from(Span::styled(lyric.text.clone(), line_style)));
        }

        // En Modo Foco usamos siempre doble espaciado para llenar la pantalla con elegancia
        lines.push(Line::from("")); 
        
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

    let alignment = Alignment::Center;

    let lyrics_widget = Paragraph::new(Text::from(lines))
        .alignment(alignment)
        .style(Style::default().bg(state.theme.bg))
        .wrap(Wrap { trim: false });

    frame.render_widget(lyrics_widget, area);
}

fn render_album_art_rect(frame: &mut Frame, state: &AppState, art_area: Rect) {
    if let Some(img) = &state.album_art {
        // Redimensionar para encajar en los caracteres (FilterType::Triangle para velocidad/calidad)
        let scaled = image::imageops::resize(img, art_area.width as u32, (art_area.height * 2) as u32, image::imageops::FilterType::Triangle);
        
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

fn render_track_info_rect(frame: &mut Frame, state: &AppState, info_area: Rect, alignment: Alignment) {
    let status_icon = if state.is_playing { "▶" } else { "⏸" };
    let title_line = Line::from(vec![
        Span::styled(&state.track_title, Style::default().fg(state.theme.bright).add_modifier(Modifier::BOLD)),
    ]);
    
    let artist_line = Line::from(Span::styled(&state.track_artist, Style::default().fg(state.theme.dim1)));

    // Fila de botones de control (visual)
    let controls_line = Line::from(vec![
        Span::styled(" [P] ", Style::default().fg(state.theme.dim2)),
        Span::styled("⏮ ", Style::default().fg(state.theme.dim1)),
        Span::styled(format!(" {} ", status_icon), Style::default().fg(state.theme.bright).add_modifier(Modifier::REVERSED)),
        Span::styled(" ⏭", Style::default().fg(state.theme.dim1)),
        Span::styled(" [N] ", Style::default().fg(state.theme.dim2)),
    ]);

    let mut lines = Vec::new();
    
    // Solo añadir espacio superior si hay altura de sobra
    if info_area.height > 5 {
        lines.push(Line::from(""));
    }
    
    lines.push(title_line);
    lines.push(artist_line);

    // Solo añadir separador si hay altura de sobra
    if info_area.height > 6 {
        lines.push(Line::from(""));
    }
    
    lines.push(controls_line);
    
    if info_area.height > 4 {
        lines.push(Line::from(vec![
            Span::styled("[Space] to ", Style::default().fg(state.theme.dim3)),
            Span::styled(if state.is_playing { "pause" } else { "play" }, Style::default().fg(state.theme.dim2)),
        ]));
    }

    let widget = Paragraph::new(lines)
        .alignment(alignment)
        .wrap(Wrap { trim: false });
        
    frame.render_widget(widget, info_area);
}

fn format_time(ms: u64) -> String {
    let seconds = ms / 1000;
    let mins = seconds / 60;
    let secs = seconds % 60;
    format!("{:02}:{:02}", mins, secs)
}

fn render_progress_bar(frame: &mut Frame, state: &AppState, area: Rect) {
    if state.duration_ms == 0 {
        return;
    }

    let pos_ms = state.interpolated_pos_ms();
    let progress = (pos_ms as f64 / state.duration_ms as f64).clamp(0.0, 1.0);
    
    let time_curr = format_time(pos_ms);
    let time_total = format_time(state.duration_ms);
    
    let text_width = 14; // "MM:SS  " y "  MM:SS" (7 cada uno)
    let width = area.width as usize;
    if width <= text_width { return; }
    
    let bar_width = width - text_width;
    let filled_width = (bar_width as f64 * progress).round() as usize;
    let mut line_spans = Vec::new();
    
    // Tiempo actual
    line_spans.push(Span::styled(format!("{}  ", time_curr), Style::default().fg(state.theme.dim1)));
    
    // Braille progress
    if filled_width > 0 {
        line_spans.push(Span::styled(
            "⣿".repeat(filled_width),
            Style::default().fg(state.theme.bright)
        ));
    }
    
    let empty_width = bar_width.saturating_sub(filled_width);
    if empty_width > 0 {
        line_spans.push(Span::styled(
            "⣀".repeat(empty_width),
            Style::default().fg(state.theme.dim3)
        ));
    }
    
    // Tiempo total
    line_spans.push(Span::styled(format!("  {}", time_total), Style::default().fg(state.theme.dim1)));

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
                // Solo reaccionar a presionar la tecla (evita dobles acciones)
                if key.kind == event::KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Char(' ') => {
                            crate::mpris::toggle_play_pause().await;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            crate::mpris::next_track().await;
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            crate::mpris::previous_track().await;
                        }
                        KeyCode::Right => {
                            crate::mpris::seek_relative(5_000_000).await;
                        }
                        KeyCode::Left => {
                            crate::mpris::seek_relative(-5_000_000).await;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Detección de tramo instrumental
        let mut target_offset = state.current_line.unwrap_or(0) as f64;
        let pos = state.interpolated_pos_ms();
        state.is_instrumental = false;
        
        if let Some(curr) = state.current_line {
            if let (Some(curr_lyric), Some(next_lyric)) = (state.lyrics.get(curr), state.lyrics.get(curr + 1)) {
                // Si la distancia total entre actual y siguiente es > 15s
                if next_lyric.timestamp_ms.saturating_sub(curr_lyric.timestamp_ms) > 15000 {
                    // Si pasaron 5s de la actual, y faltan más de 2s para la siguiente
                    if pos > curr_lyric.timestamp_ms + 5000 && pos + 2000 < next_lyric.timestamp_ms {
                        state.is_instrumental = true;
                        target_offset += 0.5; // Apuntar visualmente al medio
                    }
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
