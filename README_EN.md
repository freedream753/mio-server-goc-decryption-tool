<div align="center">

<h1>GOC Solver</h1>

**GMOD-RXSEND-MIO Server 5K Mode · GOC Rune Terminal Solver**

**noocr lite branch** (v1.1.0-noocr): a tray-resident solver for two puzzle stages — **Calibration** (draw-to-fit) and **Decryption** (four runes) — with **no screen capture / OCR / screen recognition whatsoever**. Pure math, fully offline.

[![License](https://img.shields.io/badge/LICENSE-GPL--3.0-green.svg?style=for-the-badge)](./LICENSE)
[![Release](https://img.shields.io/github/v/release/freedream753/mio-server-goc-decryption-tool?style=for-the-badge)](../../releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2B-blue.svg?style=for-the-badge)](#download)
[![Built with](https://img.shields.io/badge/built_with-Tauri%20%2B%20Rust-orange.svg?style=for-the-badge)](https://tauri.app)

[**简体中文**](./README.md) | **English**

</div>

---

## Table of Contents

- [Differences from the Mainline](#differences-from-the-mainline)
- [Features](#features)
- [How It Works](#how-it-works)
- [Download](#download)
- [Usage](#usage)
- [Technical Details](#technical-details)
- [Building from Source](#building-from-source)
- [FAQ](#faq)
- [Changelog](#changelog)
- [License & Credits](#license--credits)

## Differences from the Mainline

The mainline (`main` branch) captures the screen and uses the Windows built-in OCR to read the puzzle automatically. This branch shares the same core algorithms but **completely removes screen capture, crop mode, OCR and all screen-recognition paths**, replacing them with manual input / hand-drawn interaction — for users who don't need automatic recognition or want the tool to never touch the screen at all:

| Capability | Mainline v1.1.0 | This branch v1.1.0-noocr |
|---|---|---|
| Stage 1 Calibration | Auto-reads the waveform panel | Draw the target curve on a blank canvas → auto-fit the four sliders |
| Stage 2 Simon | Fully automatic recording | Removed |
| Stage 3 Decryption | Auto board recognition + minimax | Manual guess/color entry + same minimax engine |
| Capture / OCR / crop mode | Yes | Fully removed — the app never reads the screen |

## Features

1. **Two-stage solver:** The result window carries two tabs — Calibration and Decryption — summoned by the default hotkeys `Alt + X` (Calibration) and `Alt + V` (Decryption). Both simply open the result window on the matching tab; all remappable in settings.

2. **Calibration (stage 1 · draw-to-fit):** Hold the mouse on the blank grid canvas and trace the in-game target curve from left to right. On release the backend fits a dual sine (fundamental + second harmonic) and outputs the four slider answers (amplitude / frequency / phase / offset) plus a fitted curve for comparison. Clear-and-redraw and re-fit are supported.

3. **Decryption (stage 3 · four runes):** Pick the four runes manually and mark the feedback colors (red / yellow / green) after each submission; the candidate-first minimax strategy (first guess fixed to "empty-fire-water-wind") solves within 4 submissions. Recommendation, undo, history and a new game are all supported.

4. **Zero screen reading:** The app contains no capture, OCR, crop or screen-recognition code path at all — it never reads a single pixel from the screen and only does math. It is also smaller (x64 ≈ 30MB vs ≈ 39MB on the mainline).

5. **Focus-stealing-free result window:** Always on top, never steals game focus (WS_EX_NOACTIVATE); answers copy with a single click.

6. **Bilingual UI:** Built-in Simplified Chinese and English (including tray menu and rune names), switchable in settings.

7. **Single-file offline:** No external DLL dependencies, zero network requests (the local HTTP endpoint exists only so external scripts can summon the window).

## How It Works

```
Hotkey (Alt+X / Alt+V) or tray menu opens the result window on the matching tab
    ↓
Calibration: hold the mouse and trace the in-game target curve on the grid canvas
    ↓
Backend: even resampling → dual-sine grid search + linear least squares (R²≥0.5 check)
    ↓
Four slider answers + fitted curve preview, click to copy

Decryption: enter guesses and feedback colors manually → minimax in a Web Worker → recommended submission
```

## Download

Grab the single-file portable exe from the [Releases](../../releases) page (choose the **v1.1.0-noocr** tag):

| File | Architecture |
|---|---|
| `GOC-Solver-v1.1.0-noocr-x64.exe` | 64-bit Intel / AMD (most users) |
| `GOC-Solver-v1.1.0-noocr-x86.exe` | 32-bit x86 |
| `GOC-Solver-v1.1.0-noocr-arm64.exe` | ARM64 (e.g. Surface Pro X) |

> For automatic on-screen recognition use the mainline release [v1.1.0](../../releases/tag/v1.1.0).

## Usage

1. Run the exe — it stays in the system tray; the tray menu's "Open Solver" summons the result window and "Preferences" manages hotkeys / language / autostart
2. Calibration (stage 1): press `Alt + X`, hold the mouse on the grid canvas and trace the in-game waveform from left to right — the four slider answers appear on release; poor tracing is rejected by the R² check
3. Decryption (stage 3): press `Alt + V`, pick the four runes and mark feedback colors, submit the recommended guess and update colors until solved or a new game starts
4. Close the result window (`Esc` or the × button) — the app stays in the tray

> Requirements: Windows 10 or later; no OCR language pack needed.

## Technical Details

- **Waveform fitting kernel** (`src-tauri/src/wave.rs`): ported from the field-verified 11.9 `fit_two` — dual-sine (fundamental + second harmonic) grid search + linear least squares + coordinate-descent joint refinement; hand-drawn points are sorted by x, deduplicated and evenly resampled to 400 points before fitting so dense segments don't bias the frequency search; R²≥0.5 quality check; `to_sliders` maps back to 0–100 sliders
- **Drawing canvas** (`src/window/Result/pages.jsx`): SVG viewBox 0–100 with `preserveAspectRatio="none"`; pointer events collect a normalized point set (x∈[0,1], y∈[-1,1] with y pointing up); the canvas overlays both the hand-drawn polyline and the fitted curve
- **Recommendation engine** (`src/utils/runes.js` + Web Worker): candidate-first minimax over 1296 candidates, fully local, identical to the mainline
- **Internationalization** (`src/i18n/`): i18next with zh_CN / en_US locale packs
- **Triple-architecture builds:** x64 (x86_64-pc-windows-gnu), x86 / ARM64 (llvm-mingw gnullvm); all artifacts pass PE-header and file-version checks with WebView2Loader statically linked — true single files

## Building from Source

```bash
pnpm install
pnpm tauri build
```

> **Path requirement**: both the source tree and `CARGO_TARGET_DIR` must live on a pure-ASCII path. If the path contains non-ASCII characters (e.g. Chinese), cargo normalizes it back to the real path and the linker (ld.exe) fails to find rlib artifacts. Place the repository on an ASCII path, or point `CARGO_TARGET_DIR` at one.

## FAQ

<details>
<summary><b>Nothing happens when I press the hotkey?</b></summary>

Make sure the app is running in the tray. On this branch hotkeys only summon the result window on the matching tab — no recognition is triggered; also check other monitors in multi-display setups.
</details>

<details>
<summary><b>"Too few points" / "poor fit quality" on the Calibration tab?</b></summary>

Trace the curve across the full canvas width, from the far left to the far right, as close to the in-game target as possible (the fit requires R²≥0.5). The canvas overlays your polyline (blue) and the fitted curve (orange) — compare them and press "Re-fit".
</details>

<details>
<summary><b>The window disappeared on another monitor?</b></summary>

Dragging is restricted to the monitor where the window was summoned; to use it on another screen, close the window and press the hotkey again on that screen.
</details>

## Changelog

### v1.1.0-noocr — No-OCR lite branch

A lite branch derived from mainline v1.1.0 (`noocr` branch):

1. **OCR and screen recognition fully removed:** capture pipelines, global crop mode, crop indicator, box-selection window, WinOCR wrapper and hotkey recognition paths are all deleted; the `solver.rs` / `runes.rs` / `simon.rs` / `screenshot.rs` modules are gone and the app never reads the screen
2. **Stage 2 Simon removed:** tab, recording pipeline and the `Alt + C` hotkey are retired
3. **Stage 1 redesigned as draw-to-fit:** blank grid canvas + pointer capture + backend `fit_points` (resampling + dual-sine `fit_two` + R² check), emitting the same payload as the mainline (four sliders + curve parameters)
4. **Stage 3 keeps the core algorithm:** the frontend minimax engine is untouched; only the `rune_recognize` auto-recognition entry is removed, all input is manual
5. **Entry points adjusted:** `Alt + X` / `Alt + V` summon the result window on the matching tab; the tray menu item becomes "Open Solver"; the local HTTP endpoint now only serves `/` (summon) and `/config`
6. **Slimmer binaries:** capture/OCR dependencies removed, x64 down from 39MB to 30MB

### Previous versions (mainline)

- **v1.1.0** — three-stage solver + global crop mode (mainline latest)
- **v1.0.4** — region-select fallback in the result window, high-resolution / 4K field adaptation
- **v1.0.3** — system built-in OCR engine and topmost result window, label-driven decryption replacing the fixed shift
- **v1.0.1** — statically linked WebView2Loader for a true single-file exe; triple-architecture release convention
- **v1.0.0** — initial public release

> Full release notes at [Releases](../../releases).

## License & Credits

This project is derived from [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop), which is licensed under **GPL-3.0**. This project is therefore also released under **GPL-3.0**: you are free to use, study, modify, and distribute this software, but distributions must remain GPL-3.0, keep the original copyright and license notices, and make the complete modified source code available.

- [Tauri](https://tauri.app) — cross-platform desktop app framework
- Designed by 中分神秘博士 (Dr. Mystery of the middle part) 🧪
