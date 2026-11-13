<div align="center">

<h1>GOC Solver</h1>

**GMOD-RXSEND-MIO Server 5K Mode · GOC Rune Terminal Solver**

A tray-resident solver for all three puzzle stages — **Calibration** (waveform), **Simon** (memory sequence) and **Decryption** (four runes). One hotkey captures and solves the panel automatically, fully offline.

[![License](https://img.shields.io/badge/LICENSE-GPL--3.0-green.svg?style=for-the-badge)](./LICENSE)
[![Release](https://img.shields.io/github/v/release/freedream753/mio-server-goc-decryption-tool?style=for-the-badge)](../../releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2B-blue.svg?style=for-the-badge)](#download)
[![Built with](https://img.shields.io/badge/built_with-Tauri%20%2B%20Rust-orange.svg?style=for-the-badge)](https://tauri.app)

[**简体中文**](./README.md) | **English**

</div>

---

## Table of Contents

- [Features](#features)
- [How It Works](#how-it-works)
- [Download](#download)
- [Usage](#usage)
- [Technical Details](#technical-details)
- [Building from Source](#building-from-source)
- [FAQ](#faq)
- [Changelog](#changelog)
- [License & Credits](#license--credits)

## Features

1. **Three-stage solver:** The result window carries three tabs — Calibration, Simon and Decryption — triggered by the default hotkeys `Alt + X` (Calibration), `Alt + C` (Simon), `Alt + V` (Decryption), all remappable in settings.

2. **Calibration (stage 1 · waveform):** Automatically reads the waveform panel and gives the four slider answers (amplitude / frequency / phase / offset), plus an orange dual-sine calibration curve drawn to match the in-game target exactly.

3. **Simon (stage 2 · memory sequence):** Fully automatic — detects the Start / Playing / Repeat states, records each round's lit sequence and advances round by round; you only follow the recommendation. A manual free mode is also kept.

4. **Decryption (stage 3 · four runes):** `Alt + V` recognizes the board's runes and feedback colors (red / yellow / green) automatically; a candidate-first minimax strategy guarantees a solve within 4 submissions, with a self-learning dictionary to correct recognition errors.

5. **Global crop mode:** Toggle with the corner-frame button in the result window header. After a one-time box selection, every capture pipeline processes only that region (auto-normalized to 1920-equivalent width) with a persistent orange indicator on screen; the indicator is excluded from screen capture so it never pollutes recognition.

6. **Offline recognition:** Everything runs on the Windows built-in OCR engine and pixel geometry analysis — zero model downloads, zero network requests. A single portable exe that just works.

7. **Filter-immune detection:** Feedback colors and waveform locating use relative color-order / relative-luminance logic instead of absolute RGB thresholds, so color drift from NVIDIA Freestyle or in-game brightness filters does not break recognition.

8. **Focus-stealing-free result window:** Always on top, never steals game focus (WS_EX_NOACTIVATE) and is excluded from screen capture (WDA_EXCLUDEFROMCAPTURE); answers copy with a single click.

9. **Bilingual UI:** Built-in Simplified Chinese and English (including tray menu and rune names), switchable in settings.

## How It Works

```
Hotkey (Alt+X / Alt+C / Alt+V) triggers
    ↓
Screen capture (crop mode takes only the selected region, normalized to 1920 width)
    ↓
Stage-specific pipeline:
  · Calibration: waveform locating → structured parameter solving → slider answers + curve
  · Simon: status-region phase check (idle/playing/repeat) → lit-cell recording → step sync
  · Decryption: locate 4 rune frames → recognize runes + feedback colors → minimax next guess
    ↓
Topmost, non-activating result window shows the answer, click to copy
```

## Download

Grab the single-file portable exe from the [Releases](../../releases) page:

| File | Architecture |
|---|---|
| `GOC-Solver-v1.1.0-x64.exe` | 64-bit Intel / AMD (most users) |
| `GOC-Solver-v1.1.0-x86.exe` | 32-bit x86 |
| `GOC-Solver-v1.1.0-arm64.exe` | ARM64 (e.g. Surface Pro X) |

Single file, no external DLL dependencies, fully offline.

## Usage

1. Run the exe — it stays in the system tray; the tray menu opens the result window / settings page
2. Enable **global crop mode** first (corner-frame button in the result window header): drag a selection around the puzzle panel, and all recognition will only look at that region
3. Enter the corresponding stage and press the hotkey:
   - `Alt + X` Calibration tab: shows the four slider answers and the calibration curve
   - `Alt + C` Simon tab: fully automatic; the status area at the bottom shows the current round and the recorded sequence
   - `Alt + V` Decryption tab: recognizes the four runes and feedback colors and shows the recommended guess, click to copy; failures show a red reason below the current tab instead of switching tabs
4. If something is misread, fix it in the result window and recompute; the Decryption tab's self-learning dictionary (add / update / delete entries) keeps improving accuracy

> Requirements: Windows 10 2004 or later; OCR relies on the system's built-in Chinese language pack (zh-CN, included by default on Chinese-language Windows).

## Technical Details

- **Recognition pipelines** (`src-tauri/src/`):
  - `wave.rs` — structure-first staged waveform locating (dark threshold 30→45→60 with R²≥0.5 fallback) and full dual-sine parameter recovery
  - `simon.rs` — status-region phase machine (idle / playing / repeat) with pure-pixel fast checks + a dedicated lit-cell capture thread, zero OCR cost during playback; 1-frame-dark clearing + 300ms refractory period supports same-cell double blinks
  - `runes.rs` — rune frame locating (low-watermark vertical-band bridging) → relative border color-order feedback detection → template matching with OCR fallback for glyphs
  - `solver.rs` — candidate-first minimax over 1296 candidates, first guess fixed to "empty-fire-water-wind", guaranteed solve within 4 submissions
- **Global crop mode** (`window.rs` / `cmd.rs`): selection supports negative global coordinates (side monitors) and survives restart; hotkeys without crop mode first open a box-selection window and auto-dispatch after selection; all persistent windows are excluded from screen capture
- **Recommendation engine** (`src/utils/runes.js` + Web Worker): fully local offline next-guess computation
- **Internationalization** (`src/i18n/`): i18next with zh_CN / en_US locale packs; the Rust side only returns error codes, translations are rendered by the frontend per language
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

Make sure the app is running in the tray, and that the game runs in borderless-windowed mode (not exclusive fullscreen) — the topmost window is invisible in exclusive fullscreen. If global crop mode has never been enabled, the hotkey first opens a box-selection window and runs the recognition automatically after you select the puzzle area.
</details>

<details>
<summary><b>Recognition failed / red error text?</b></summary>

The failure reason appears in red right below the current tab (e.g. "capture failed", "rune frames not found"). First check that the global crop selection covers the full puzzle panel; on 4K / high-DPI, raising the in-game UI scale significantly improves recognition.
</details>

<details>
<summary><b>Recognition breaks with filters enabled?</b></summary>

Detection uses relative color-order / relative-luminance logic, which already adapts to luminance-type filters such as NVIDIA Freestyle or in-game brightness boosts. If a strong tint filter (e.g. night mode) still causes issues, please open an issue with a full-screen screenshot attached.
</details>

<details>
<summary><b>The window disappeared on another monitor?</b></summary>

Dragging is restricted to the monitor where the window was summoned; to use it on another screen, close the window and press the hotkey again on that screen.
</details>

## Changelog

### v1.1.0 — Three-stage solver + global crop mode

All changes since v1.0.4 are merged into this release:

1. **Calibration / Simon / Decryption assistants:** the result window becomes a three-tab layout; `Alt + X` / `Alt + C` / `Alt + V` trigger each stage's automatic recognition and solving, all integrated into one window
2. **Fully automatic Simon mode:** state-machine driven (idle / playing / repeat), auto-detects the start button, records each round's lit sequence and advances by round with zero OCR cost during playback; supports same-cell double blinks, off-window pollution defense and 3+X round truncation
3. **Decryption strategy upgrade:** the core algorithm is replaced with a minimax implementation (candidate-first, gamble on direct hit) — no probe guesses, first guess fixed to "empty-fire-water-wind", guaranteed solve within 4 submissions
4. **Global crop mode:** after one box selection, all capture pipelines process only that region, normalized to 1920-equivalent width; the orange indicator is capture-excluded (WDA_EXCLUDEFROMCAPTURE); hotkeys without crop mode open a box-selection window first and auto-dispatch
5. **Bilingual UI:** Simplified Chinese + English (tray menu and rune names included), with the other 17 locale packs removed; rune recognition failures are localized by error code
6. **Stability fixes:** same-cell double blinks (1-frame-dark clearing + 300ms refractory), per-stage recording targets, serialized screenshot IO, generational-guard full refetch for simon steps, atomic AFTER_CROP handling, Win32 visibility check for the waveform window, failures no longer switch tabs, and more

### Previous versions

- **v1.0.4** — region-select fallback in the result window, high-resolution / 4K field adaptation
- **v1.0.3** — system built-in OCR engine and topmost result window, label-driven decryption replacing the fixed shift
- **v1.0.1** — statically linked WebView2Loader for a true single-file exe; triple-architecture release convention
- **v1.0.0** — initial public release

> Historical note: the v1.0.x cipher-recognition + Caesar-shift pipeline (for the legacy GOC puzzle) was superseded by the three-stage solver and fully removed during the v1.1.0 development cycle. Full release notes at [Releases](../../releases).

## License & Credits

This project is derived from [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop), which is licensed under **GPL-3.0**. This project is therefore also released under **GPL-3.0**: you are free to use, study, modify, and distribute this software, but distributions must remain GPL-3.0, keep the original copyright and license notices, and make the complete modified source code available.

- [Tauri](https://tauri.app) — cross-platform desktop app framework
- [Windows.Media.Ocr](https://learn.microsoft.com/windows/uwp/audio-video-camera/imaging) — Windows built-in OCR
- Designed by 中分神秘博士 (Dr. Mystery of the middle part) 🧪
