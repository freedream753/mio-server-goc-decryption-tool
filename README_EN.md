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
- [License & Credits](#license--credits)

## Features

1. **Fully automatic solving:** One keypress (`Alt + C`) captures the full screen, locates the three cipher cards automatically, and completes recognition → decryption → copy in one go — no manual region selection needed.

2. **Offline recognition:** Uses the Windows built-in OCR engine (Windows.Media.Ocr) — zero model downloads, zero network requests. A single portable exe that just works.

3. **Filter-immune color detection:** Color decisions are based on relative color ordering (channel differences + cross-color relative order) instead of absolute RGB thresholds, so global color drift caused by NVIDIA Freestyle / in-game brightness filters does not break recognition.

4. **Focus-stealing-free result window:** The result window is always on top and never steals game focus (WS_EX_NOACTIVATE). Click any answer row to copy it without interrupting your keystrokes in the game.

5. **Self-learning corrections:** If a recognition is wrong, edit the answer directly in the result window. Corrections are saved to a local JSON file and preferred in future recognition — fix once, remembered forever.

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
| `GOC-Solver-v1.0.3-x64.exe` | 64-bit Intel / AMD |
| `GOC-Solver-v1.0.3-x86.exe` | 32-bit x86 |
| `GOC-Solver-v1.0.3-arm64.exe` | ARM64 |

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

## License & Credits

This project is derived from [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop), which is licensed under **GPL-3.0**. This project is therefore also released under **GPL-3.0**: you are free to use, study, modify, and distribute this software, but distributions must remain GPL-3.0, keep the original copyright and license notices, and make the complete modified source code available.

- [Tauri](https://tauri.app) — cross-platform desktop app framework
- [Windows.Media.Ocr](https://learn.microsoft.com/windows/uwp/audio-video-camera/imaging) — Windows built-in OCR
- Designed by 中分神秘博士 (Dr. Mystery of the middle part) 🧪
