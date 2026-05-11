use std::io::{self, Write};
use std::time::Instant;
use tokio::sync::mpsc::Receiver;

use crate::{
    lyrics::{find_current_line, LrcLine},
    AppEvent,
};

struct RawState {
    lyrics: Vec<LrcLine>,
    last_known_pos_ms: u64,
    last_sync_time: Instant,
    current_line: Option<usize>,
    lyrics_loading: bool,
    is_playing: bool,
    initial_sync_done: bool,
}

impl RawState {
    fn new() -> Self {
        Self {
            lyrics: Vec::new(),
            last_known_pos_ms: 0,
            last_sync_time: Instant::now(),
            current_line: None,
            lyrics_loading: true,
            is_playing: false,
            initial_sync_done: false,
        }
    }

    fn interpolated_pos_ms(&self) -> u64 {
        if self.is_playing {
            self.last_known_pos_ms + self.last_sync_time.elapsed().as_millis() as u64
        } else {
            self.last_known_pos_ms
        }
    }

    fn handle_event(&mut self, event: AppEvent) -> bool {
        let mut changed = false;
        match event {
            AppEvent::Position(pos_us) => {
                if !self.initial_sync_done {
                    let new_pos_ms = (pos_us.max(0) / 1000) as u64;
                    self.initial_sync_done = true;
                    self.last_known_pos_ms = new_pos_ms;
                    self.last_sync_time = Instant::now();
                    changed = true;
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
                changed = true;
            }
            AppEvent::Tick => {
                let pos = self.interpolated_pos_ms();
                let new_line = find_current_line(&self.lyrics, pos);
                if new_line != self.current_line {
                    self.current_line = new_line;
                    changed = true;
                }
            }
            AppEvent::TrackChanged(_info) => {
                self.lyrics.clear();
                self.lyrics_loading = true;
                self.current_line = None;
                self.last_known_pos_ms = 0;
                self.last_sync_time = Instant::now();
                changed = true;
            }
            AppEvent::Lyrics(lines) => {
                self.lyrics_loading = false;
                self.lyrics = lines;
                self.current_line = None;
                changed = true;
            }
            AppEvent::ArtProcessed(_, _) => {}
            AppEvent::Quit => {}
        }
        changed
    }
}

pub async fn run(mut rx: Receiver<AppEvent>) -> anyhow::Result<()> {
    let mut state = RawState::new();

    // Loop que imprime a stdout cada vez que cambia la línea actual
    loop {
        // Drenar eventos
        let mut updated = false;
        if let Some(event) = rx.recv().await {
            if matches!(event, AppEvent::Quit) {
                break;
            }
            updated |= state.handle_event(event);
        } else {
            break; // Canal cerrado
        }

        // Drenar eventos pendientes sin bloquear
        while let Ok(event) = rx.try_recv() {
            if matches!(event, AppEvent::Quit) {
                return Ok(());
            }
            updated |= state.handle_event(event);
        }

        if updated {
            if state.lyrics.is_empty() {
                if state.lyrics_loading {
                    // No imprimimos nada mientras carga para no spamear
                } else {
                    println!("(no lyrics)");
                }
            } else {
                let current = state.current_line.unwrap_or(0);
                if current < state.lyrics.len() {
                    let text = &state.lyrics[current].text;
                    if text.trim().is_empty() {
                        println!("...");
                    } else {
                        println!("{}", text);
                    }
                }
            }
            // Forzar flush para que herramientas como Waybar lo lean de inmediato
            let _ = io::stdout().flush();
        }
    }

    Ok(())
}
