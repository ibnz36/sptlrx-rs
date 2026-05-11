# sptlrx-rs
<img width="1920" height="1080" alt="image" src="https://github.com/user-attachments/assets/00b45d61-9871-4d93-9fbc-1f08816db7e6" />


A high-performance, low-latency Spotify synchronized lyrics client for Linux terminal environments.

Built in Rust, `sptlrx-rs` leverages the MPRIS D-Bus interface and `tokio` asynchronous runtime to provide exact lyric synchronization without relying on Spotify's rate-limited Web API. The UI is designed to be completely borderless and distraction-free, making it ideal for integration into Wayland compositors (e.g., Hyprland) as a transparent floating widget.
<img width="1091" height="642" alt="image" src="https://github.com/user-attachments/assets/ffb69c9f-6d1f-44dd-8f3f-cdeb4aba5afe" />


## Architecture

The system is built on a multi-actor model using `mpsc` channels to ensure the render loop is never blocked by network or IPC latency:

- **MPRIS Actor (`zbus`)**: Listens to the `org.mpris.MediaPlayer2.Player` interface. It polls the `Position` property and listens to the `Seeked` signal to handle track changes and scrubbing instantly.
- **Ticker Actor**: Emits 60Hz pulses to interpolate the playback position internally. This avoids D-Bus polling saturation while maintaining fluid visual synchronization.
- **Fetcher Actor (`reqwest`)**: Cleans Spotify track metadata (stripping noise like "Remastered" or "Live") and resolves `.lrc` files via the LRCLIB API. Supports concurrent cancellation if the user skips tracks rapidly.
- **Render Actor (`ratatui`)**: A strictly minimalist TUI that renders only the currently active lyric line. Computes vertical/horizontal constraints dynamically to achieve a true full-screen, borderless layout.

## Dependencies

- Rust toolchain (edition 2024 / rustc 1.91+)
- D-Bus (user session)
- OpenSSL / rustls

## Build & Run

```bash
cargo build --release
./target/release/sptlrx-rs
```

## Window Manager Integration (Hyprland / Wayland)

The UI expects a transparent terminal background. To use it as a desktop widget, configure your terminal emulator to load a large font size and set a window rule in your compositor to float the window without borders.

Example `kitty` invocation:
```bash
kitty -o font_size=42 -o font_family="JetBrainsMono Nerd Font" --class sptlrx-widget -e ./target/release/sptlrx-rs
```

Example `hyprland.conf` rules:
```hyprlang
windowrulev2 = float, class:^(sptlrx-widget)$
windowrulev2 = noborder, class:^(sptlrx-widget)$
windowrulev2 = center, class:^(sptlrx-widget)$
```
