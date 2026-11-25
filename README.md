# Text Extractor (Rust)

A lightweight, standalone Windows OCR screen-capture utility written in pure Rust (`~350 KB` single binary, `~11 MB` RAM), inspired by Microsoft PowerToys Text Extractor and powered by the native `Windows.Media.Ocr` engine.

## Features
- **Native Windows OCR (`Windows.Media.Ocr`)**: Supports all Windows OCR language packs (Arabic, English, etc.).
- **Floating Top Toolbar**:
  - Live OCR language switcher dropdown
  - **Normal Mode** (preserve lines)
  - **Single Line Mode** (`S` key)
  - **Table Mode** (`T` key, tab-separated columns for Excel)
  - **Change Shortcut** dialog (`⚙` → *Change Shortcut...*)
- **Smooth Selection**:
  - Drag to select any region, or single-click a word
  - Hold `Shift` while dragging to move the selection box
  - High-contrast crosshair cursor
  - `Esc` or right-click to cancel
- **Global Hotkey & System Tray**: Default `Win + Shift + T` (customizable), saved in `config.txt`.

## Build from Source
```powershell
cargo build --release
```
The compiled binary will be at `target/release/TextExtractor.exe`.
