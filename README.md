# sptlrx-rs

<img width="1920" height="1080" alt="image" src="https://github.com/user-attachments/assets/00b45d61-9871-4d93-9fbc-1f08816db7e6" />

A high-performance, low-latency Spotify synchronized lyrics client for Linux terminal environments.

Built in Rust, `sptlrx-rs` leverages the MPRIS D-Bus interface and `tokio` asynchronous runtime to provide exact lyric synchronization without relying on Spotify's rate-limited Web API. The UI is designed to be completely borderless and distraction-free, making it ideal for integration into Wayland compositors (e.g., Hyprland) as a transparent floating widget.

<img width="1091" height="642" alt="image" src="https://github.com/user-attachments/assets/ffb69c9f-6d1f-44dd-8f3f-cdeb4aba5afe" />

<p align="center">
  <img width="965" src="https://github.com/user-attachments/assets/4d2f0ba8-8ad4-4808-b4ba-687c93f9dd74" alt="sptlrx-rs showcase" />
</p>

## Features

- **Zero API Latency:** Reads playback status directly from your OS via D-Bus/MPRIS. No Spotify developer tokens required.
- **Dynamic Theming:** Automatically extracts the dominant color from the current Spotify album art to style the TUI in real-time.
- **Smooth Interpolation:** Calculates track position internally at 60fps for buttery smooth lyric scrolling without spamming D-Bus.
- **Rice-Ready (Raw Mode):** Outputs current lyric lines directly to `stdout`. Perfect for feeding into Waybar, Polybar, Eww, or AGS.
- **Featherweight:** Compiled static binary written in pure Rust (TLS via `rustls`). Negligible CPU and RAM footprint.

## Architecture

The system is built on a multi-actor model using `mpsc` channels to ensure the render loop is never blocked by network or IPC latency:

- **MPRIS Actor (`zbus`)**: Listens to the `org.mpris.MediaPlayer2.Player` interface. It polls the `Position` property and listens to the `Seeked` signal to handle track changes and scrubbing instantly.
- **Ticker Actor**: Emits 60Hz pulses to interpolate the playback position internally. This avoids D-Bus polling saturation while maintaining fluid visual synchronization.
- **Fetcher Actor (`reqwest`)**: Cleans Spotify track metadata (stripping noise like "Remastered" or "Live") and resolves `.lrc` files via the LRCLIB API. Supports concurrent cancellation if the user skips tracks rapidly.
- **Render Actor (`ratatui`)**: A strictly minimalist TUI that renders only the currently active lyric line. Computes vertical/horizontal constraints dynamically to achieve a true full-screen, borderless layout.

## Dependencies

- Rust toolchain (edition 2024 / rustc 1.91+)
- D-Bus (user session)

*(Note: `sptlrx-rs` uses `rustls` for cryptography, meaning it does not require C-based OpenSSL libraries to compile).*

## Installation

### Nix / NixOS (Recommended)
If you are using Nix with flakes enabled, you can run it directly without installing anything:
```bash
nix run github:tu-usuario/sptlrx-rs
```
Or build the static MUSL binary:
```bash
nix build github:tu-usuario/sptlrx-rs#static
```

### Cargo (Other Linux Distros)
```bash
git clone [https://github.com/tu-usuario/sptlrx-rs](https://github.com/tu-usuario/sptlrx-rs)
cd sptlrx-rs
cargo build --release
./target/release/sptlrx-rs
```

## Configuration & Theming

`sptlrx-rs` supports custom themes and solid backgrounds. By default, it looks for a configuration file at `~/.config/sptlrx-rs/config.toml`.

Example `config.toml`:
```toml
# Built-in themes: "catppuccin-mocha" (default), or "custom"
theme = "custom"
# Optional: Force a solid background instead of transparency
# background = "#0d1117"

[custom]
accent = "#58a6ff"
text = "#c9d1d9"
dim1 = "#8b949e"
dim2 = "#484f58"
dim3 = "#21262d"
bar = "#1f6feb"
```
> **Note:** If the TUI successfully fetches album art from Spotify, it will dynamically extract the dominant color and override the `accent` and `bar` colors automatically!

## Status Bar Integration (Raw Mode)

If you want to pipe the currently sung lyric line directly into your status bar, use the `--raw` flag. This bypasses the graphical interface completely and prints plain text to `stdout` as the song progresses.

```bash
sptlrx-rs --raw
```

## Window Manager Integration (Hyprland / Wayland)

The UI expects a transparent terminal background. To use it as a desktop widget, configure your terminal emulator to load a large font size and set a window rule in your compositor to float the window without borders.

Example `kitty` invocation:
```bash
kitty -o font_size=42 -o font_family="JetBrainsMono Nerd Font" --class sptlrx-widget -e sptlrx-rs
```

Example `hyprland.conf` rules:
```hyprlang
windowrulev2 = float, class:^(sptlrx-widget)$
windowrulev2 = noborder, class:^(sptlrx-widget)$
windowrulev2 = center, class:^(sptlrx-widget)$
```
