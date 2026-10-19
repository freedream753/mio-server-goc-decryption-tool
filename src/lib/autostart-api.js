// tauri-plugin-autostart v1 JS API 本地实现（替代 github:tauri-apps/tauri-plugin-autostart#v1）
import { invoke } from '@tauri-apps/api/tauri';

export async function isEnabled() {
    return await invoke('plugin:autostart|is_enabled');
}

export async function enable() {
    await invoke('plugin:autostart|enable');
}

export async function disable() {
    await invoke('plugin:autostart|disable');
}
