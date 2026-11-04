<div align="center">

<h1>GOC Solver</h1>

**GMOD-RXSEND-MIO Server 5K Mode · GOC Cipher Terminal Solver**

Automatically recognizes the three cipher cards on the puzzle page, decrypts them according to each card's "shift forward / backward by N" label, and shows the answers in a topmost window with one-click copy.

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

1. **Fully automatic solving:** One keypress (`Alt + C`) captures the full screen, locates the three cipher cards automatically, and completes recognition → decryption → copy in one go — no manual region selection needed.

2. **Offline recognition:** Uses the Windows built-in OCR engine (Windows.Media.Ocr) — zero model downloads, zero network requests. A single portable exe that just works.

3. **Filter-immune color detection:** Color decisions are based on relative color ordering (channel differences + cross-color relative order) instead of absolute RGB thresholds, so global color drift caused by NVIDIA Freestyle / in-game brightness filters does not break recognition.

4. **Focus-stealing-free result window:** The result window is always on top and never steals game focus (WS_EX_NOACTIVATE). Click any answer row to copy it without interrupting your keystrokes in the game.

5. **Self-learning corrections:** If a recognition is wrong, edit the answer directly in the result window. Corrections are saved to a local JSON file and preferred in future recognition — fix once, remembered forever.

6. **Region-select from the result window:** A box-select button in the result window header (the four-corner icon right of ⠿) lets you drag-select any screen area for recognition — a fallback for high-resolution / high-DPI scenes where automatic locating fails. Small selections are upscaled automatically.

## How It Works

```
Alt + C full-screen capture
    ↓
Rust pixel pre-filter (luminance channel + card-border geometry) locates the three cards
    ↓
Windows.Media.Ocr reads the "shift forward/backward by N" labels + uppercase ciphers
    ↓
Decrypt per label semantics (backward N → each letter -N; forward N → each letter +N)
    ↓
Topmost, non-activating result window: three "cipher → answer" rows, first answer copied by default
```

## Download

Grab the single-file portable exe from the [Releases](../../releases) page:

| File | Architecture |
|---|---|
| `GOC-Solver-v1.0.4-x64.exe` | 64-bit Intel / AMD |
| `GOC-Solver-v1.0.4-x86.exe` | 32-bit x86 |
| `GOC-Solver-v1.0.4-arm64.exe` | ARM64 |

Single file, no external DLL dependencies, fully offline.

## Usage

1. Run the exe — it stays in the system tray
2. Open the GOC cipher terminal puzzle page and press `Alt + C`
3. The result window shows three "cipher → answer" rows; click any row to copy that answer
4. The window auto-hides after a 10-second countdown (clicking to copy or dragging the window resets the timer; the countdown freezes while dragging)
5. If something is misrecognized, edit the answer in the result window — the correction is saved automatically

> Requirements: Windows 10 or later; recognition relies on the system's built-in Chinese OCR language pack (zh-CN, included by default on Chinese-language Windows).

## Technical Details

- **Recognition pipeline** (`src-tauri/src/solver.rs`): luminance-channel binarization + row detection to locate cards; tight ROI cropping to work around Windows OCR returning empty strings for wide images; grayscale normalization (background→white, text→black, anti-aliasing preserved) to boost accuracy, with hard binarization as fallback
- **Decryption semantics:** backward N → each letter -N, forward N → each letter +N; a small built-in word list is used only for answer verification hints (✓ matched / ？unverified)
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
<summary><b>Empty or garbled results?</b></summary>

Make sure the Chinese language pack is installed (Settings → Time & Language → Language → Add Chinese). If only a few characters are wrong, just fix them in the result window — corrections are remembered.
</details>

<details>
<summary><b>Recognition breaks with filters enabled?</b></summary>

Since v1.0.3, color detection adapts to luminance-type filters such as NVIDIA Freestyle / in-game brightness boosts. If a strong tint filter (e.g. night mode) still causes issues, please open an issue with a full-screen screenshot attached.
</details>

<details>
<summary><b>Failing on 4K / high-resolution, or an "interface too small" hint?</b></summary>

Since v1.0.4, small UIs on high-resolution screens are adapted automatically (small-UI parameter set + OCR scale derived from measured glyph height). If it still fails, use the box-select button in the result window header to drag-select the full puzzle dialog (labels, ciphers and input box included) — small selections are upscaled 2x before recognition. Raising the in-game UI scale also helps a lot.
</details>

## Changelog

### v1.0.4 (2026-11-04) — Region-Select Returns + High-Res / 4K Adaptation

1. **Region-select from the result window:** New four-corner button next to ⠿ in the header — click to hide the result window, drag-select a full-screen region, recognize, and get results back in the result window; the v1.0.1 manual selection capability returns
2. **Small-selection upscaling:** Selections smaller than 1100×700 are upscaled 2x (CatmullRom) before entering the pipeline, bringing 4K@100% small text back into the tuned size range
3. **Cipher-only solving:** When no "shift forward / backward" label is present, the cipher is read directly via column-projection segmentation, with a three-level fallback: learned corrections → built-in word list → manual ± buttons
4. **4K field localization fix:** Low-watermark + 70-row bridging extension for vertical bands, re-joining dialog bands torn apart by dark input-box rows
5. **Degraded fallback for failed localization:** When automatic locating fails, reads bands directly within the dialog area using a blue-dark mask + row-band high-pass, keeping only high-confidence results
6. **High-resolution small-UI adaptation:** Screens ≥2560 px wide automatically retry with a small-UI parameter set; OCR magnification (2–8x) is derived from measured glyph height when the input box is too small
7. **Fixed near-white pixel misdetection:** Color-detection channel arithmetic changed from u8 to u16, eliminating addition wrap-around that misread near-white pixels as dark blue — the root cause of failures under white night-vision filters

### v1.0.3 (2026-11-03) — New Puzzle Page Adaptation

Full rewrite after the game updated to a three-card puzzle page:

1. **Fully automatic recognition:** `Alt + C` captures the full screen and locates the three cipher cards automatically — no manual region selection anymore
2. **OCR engine swap:** Tesseract.js (wasm models) retired in favor of the Windows built-in Windows.Media.Ocr engine — zero model downloads, fully offline, smaller exe
3. **Label-based decryption:** Reads each card's "shift forward / backward by N" label and decrypts accordingly (backward → -N, forward → +N), replacing the fixed Caesar -3
4. **New result window:** Always on top without stealing focus, three "cipher → answer" rows, click to copy, auto-hides after a 10-second countdown
5. **Self-learning:** Corrections made in the result window are remembered permanently — if the game's semantics ever change, one fix is enough
6. **Filter adaptation:** Relative color-order detection + grayscale normalization, compatible with color drift from NVIDIA Freestyle / in-game brightness filters
7. Cipher word-length limit extended to 3–12 letters; the old manual-selection pipeline is retired
8. **Field localization hardening:** Adaptive column-projection thresholds, layered fallback + equal-width cluster validation for the input box, and cross-cluster consensus for cipher bands — fixing recognition failures in real-world scenes such as dimmed holograms and cursor hover overlap

### v1.0.1 (2026-10-27)

1. OCR logic tuning — 100% accuracy on both regression screenshots
2. Statically linked WebView2Loader for a true single-file exe (no bundled DLLs)
3. Fixed blank windows caused by missing `custom-protocol` build feature, broken logo, etc.
4. Completed x86 / ARM64 artifacts; triple-architecture release convention established

### v1.0.0 (2026-10-19) — Initial Public Release

1. Manual region selection → Tesseract.js OCR → Caesar -3 decryption → auto copy to clipboard
2. Single-file exe for x64 / x86 / ARM64

> v1.0.2 was an internal build (failure logging + tray "View Log" entry), never separately released; its content is included in v1.0.3. Full release notes at [Releases](../../releases).

## License & Credits

This project is derived from [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop), which is licensed under **GPL-3.0**. This project is therefore also released under **GPL-3.0**: you are free to use, study, modify, and distribute this software, but distributions must remain GPL-3.0, keep the original copyright and license notices, and make the complete modified source code available.

- [Tauri](https://tauri.app) — cross-platform desktop app framework
- [Windows.Media.Ocr](https://learn.microsoft.com/windows/uwp/audio-video-camera/imaging) — Windows built-in OCR
- Designed by 中分神秘博士 (Dr. Mystery of the middle part) 🧪
