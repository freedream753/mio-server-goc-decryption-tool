// tauri-plugin-fs-watch v1 JS API 本地实现（替代 github:tauri-apps/tauri-plugin-fs-watch#v1）
import { invoke } from '@tauri-apps/api/tauri';
import { appWindow } from '@tauri-apps/api/window';

const w = appWindow;

export async function watch(paths, cb, options = {}) {
    const opts = { recursive: false, delayMs: 2000, ...options };
    const watchPaths = typeof paths === 'string' ? [paths] : paths;
    const id = window.crypto.getRandomValues(new Uint32Array(1))[0];
    await invoke('plugin:fs-watch|watch', { id, paths: watchPaths, options: opts });
    const unlisten = await w.listen(`watcher://debounced-event/${id}`, (event) => {
        cb(event.payload);
    });
    return () => {
        void invoke('plugin:fs-watch|unwatch', { id });
        unlisten();
    };
}

export async function watchImmediate(paths, cb, options = {}) {
    const opts = { recursive: false, ...options, delayMs: null };
    const watchPaths = typeof paths === 'string' ? [paths] : paths;
    const id = window.crypto.getRandomValues(new Uint32Array(1))[0];
    await invoke('plugin:fs-watch|watch', { id, paths: watchPaths, options: opts });
    const unlisten = await w.listen(`watcher://raw-event/${id}`, (event) => {
        cb(event.payload);
    });
    return () => {
        void invoke('plugin:fs-watch|unwatch', { id });
        unlisten();
    };
}
