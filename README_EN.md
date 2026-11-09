<div align="center">

<h1>GOC Solver</h1>

**GMOD-RXSEND-MIO Server 5K Mode · GOC Rune Terminal Solver**

Press `Alt + C` to capture the full screen, automatically recognize the four runes on the game panel and their feedback colors (red / yellow / green), record them into this round's history, and show the next recommended guess in a topmost window.

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

1. **Fully automatic rune recognition:** One keypress (`Alt + C`) captures the full screen, locates the four rune slots of the current guess, recognizes each rune (empty / fire / water / wind / earth / thunder) and each feedback color (red / yellow / green), and records the round automatically — no manual entry.

2. **Offline recognition:** Rune locating, border color sampling and glyph recognition all run locally — zero model downloads, zero network requests. A single portable exe that just works.

3. **Filter-immune color detection:** Feedback colors are decided by the relative color order of each rune frame's border instead of absolute RGB thresholds, so global color drift caused by NVIDIA Freestyle / in-game brightness filters does not break recognition.

4. **Focus-stealing-free result window:** The result window is always on top and never steals game focus (WS_EX_NOACTIVATE). It shows the recognized panel and the next recommended guess, ready to copy with one click without interrupting your keystrokes.

5. **Per-slot correction:** If a rune or color is misread, fix it slot by slot in the result window and the recommendation is recomputed; "undo this entry" and "new round" are available as a safety net.

6. **Manual helper & region-select fallback:** The "Four Runes" window (tray / settings sidebar) stays for switching the color rule, entering history manually and joining mid-round; the box-select button in the result window recognizes a manually dragged screen region when automatic locating fails.

## How It Works

```
Alt + C full-screen capture
    ↓
Locate the four rune slots of the current guess (panel luminance / frame geometry)
    ↓
Recognize 4 runes (empty / fire / water / wind / earth / thunder) + 4 feedback colors (red / yellow / green)
    ↓
Deduplicate against the current round, then append one history entry (localStorage key goc-rune-session-v1)
    ↓
Topmost, non-activating result window: recognized panel + next recommended guess, one-click copy
```

## Download

Grab the single-file portable exe from the [Releases](../../releases) page:

| File | Architecture |
|---|---|
| `GOC-Solver-v1.1.0-x64.exe` | 64-bit Intel / AMD |
| `GOC-Solver-v1.1.0-x86.exe` | 32-bit x86 |
| `GOC-Solver-v1.1.0-arm64.exe` | ARM64 |

Single file, no external DLL dependencies, fully offline.

## Usage

1. Run the exe — it stays in the system tray
2. Open the GOC rune terminal page and, for a new round, submit the recommended "empty, fire, water, wind" first
3. After the game returns feedback, press `Alt + C`: the four runes and their feedback colors are recognized and recorded into this round
4. The result window shows the recognized panel plus the next recommended guess; click the recommendation to copy it, and the round is reported as solved once everything is green
5. The window auto-hides after a 10-second countdown (clicking to copy or dragging the window resets the timer; the countdown freezes while dragging)
6. If something was misread, fix the runes / colors slot by slot in the result window and the recommendation is recomputed, or use "undo this entry"; switch the color rule or enter history manually in the "Four Runes" window

> Requirements: Windows 10 or later; glyph recognition relies on the system's built-in Chinese OCR language pack (zh-CN, included by default on Chinese-language Windows).

## Technical Details

- **Rune recognition pipeline** (`src-tauri/src/`): full-screen capture → locate the four rune frames → sample the frame border to decide the feedback color → recognize the single-character rune; any slot that cannot be recognized is left empty and corrected in the result window
- **Recommendation engine** (`src/utils/runes.js` + Web Worker): the 1296 candidate answers are filtered by the color rule (presence = yellow / count-based), yielding a next guess that is guaranteed to finish within four submissions
- **Round history** (`src/utils/rune-session.js`): localStorage key `goc-rune-session-v1`, shared by the hotkey pipeline and the manual helper
- **Result window** (`src-tauri/src/window.rs`): Tauri v1 + windows-rs adds the WS_EX_NOACTIVATE / SW_SHOWNA Win32 calls; the countdown freezes while the window is being dragged
- **Triple-architecture builds:** x64 (x86_64-pc-windows-gnu), x86 / ARM64 (llvm-mingw gnullvm); all artifacts pass PE-header and DLL import-table checks (system libraries only)

## Building from Source

```bash
pnpm install
pnpm tauri build
```

For build notes and pitfalls, see [PLAN-v1.0.3-新版解题页面适配.md](./PLAN-v1.0.3-新版解题页面适配.md) (Chinese).

## FAQ

<details>
<summary><b>Nothing happens when I press Alt + C?</b></summary>

Make sure the app is running in the tray, and that the game runs in borderless-windowed mode (not exclusive fullscreen) — the topmost window is invisible in exclusive fullscreen.
</details>

<details>
<summary><b>A rune or feedback color was misread?</b></summary>

The result window lists the four recognized runes and colors — click a slot to change it to the correct rune / color and the recommendation is recomputed automatically, or use "undo this entry" and retry. If the whole panel was misread, use the box-select button in the result window to drag-select the rune panel and recognize it again.
</details>

<details>
<summary><b>Recognition breaks with filters enabled?</b></summary>

Feedback colors are judged by the relative order of frame border colors, which already adapts to luminance-type filters such as NVIDIA Freestyle / in-game brightness boosts. If a strong tint filter (e.g. night mode) still causes issues, please open an issue with a full-screen screenshot attached.
</details>

<details>
<summary><b>Failing on 4K / high-resolution, or an "interface too small" hint?</b></summary>

Recognition scales adaptively from the measured glyph height; if it still fails, use the box-select button in the result window to drag-select the full rune panel (all four runes and their frames) — small selections are upscaled before recognition. Raising the in-game UI scale also helps a lot.
</details>

## Changelog

### v1.1.0 — Hotkey now runs automatic rune recognition

1. **`Alt + C` re-wired:** full-screen capture → locate the four rune slots of the current guess → recognize 4 runes and 4 feedback colors → append the entry to this round's history → topmost window shows the next recommended guess
2. **The tray "capture & recognize runes" entry runs the same pipeline:** hotkey, tray menu and the result window's box-select button all share one automatic flow
3. **The result window now carries rune results:** recognized panel, recommended guess, per-slot correction, undo-entry and new-round
4. **The manual helper is kept:** the "Four Runes" window keeps handling color-rule switching, manual entry and joining mid-round
5. **The old cipher pipeline is gone:** card pixel locating, cipher-label parsing, shift decryption, the answer word list and self-learning corrections, plus the retired wasm OCR front-end and its dependencies

### Previous versions

- **v1.0.4** — region-select fallback in the result window, high-resolution / 4K field adaptation
- **v1.0.3** — system built-in OCR engine and topmost result window, label-driven decryption replacing the fixed shift
- **v1.0.1** — statically linked WebView2Loader for a true single-file exe; triple-architecture release convention
- **v1.0.0** — initial public release

> Historical note: the v1.0.x cipher-decryption pipeline was removed entirely in v1.1.0 and no entry point for it remains. Full release notes at [Releases](../../releases).

## License & Credits

This project is derived from [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop), which is licensed under **GPL-3.0**. This project is therefore also released under **GPL-3.0**: you are free to use, study, modify, and distribute this software, but distributions must remain GPL-3.0, keep the original copyright and license notices, and make the complete modified source code available.

- [Tauri](https://tauri.app) — cross-platform desktop app framework
- [Windows.Media.Ocr](https://learn.microsoft.com/windows/uwp/audio-video-camera/imaging) — Windows built-in OCR
- Designed by 中分神秘博士 (Dr. Mystery of the middle part) 🧪
