// tauri-plugin-fs-watch v1 JS API 本地实现（替代 github:tauri-apps/tauri-plugin-fs-watch#v1）
import { invoke } from '@tauri-apps/api/tauri';
import { appWindow } from '@tauri-apps/api/window';

const w = appWindow;

// 路径哈希生成确定性 id（FNV-1a）：插件把监听器存在应用级集合且不随窗口销毁清理，
// 同路径反复注册会线性泄漏监听器与其工作线程（反复开关设置窗口实测每轮 +2~4 线程）。
// 固定 id + 注册前先反注册旧监听，使同路径全局只保留一个监听器。
function pathsId(paths) {
    const s = paths.join('|');
    let h = 0x811c9dc5;
    for (let i = 0; i < s.length; i++) {
        h ^= s.charCodeAt(i);
        h = Math.imul(h, 0x01000193) >>> 0;
    }
    return h >>> 0;
}

export async function watch(paths, cb, options = {}) {
    const opts = { recursive: false, delayMs: 2000, ...options };
    const watchPaths = typeof paths === 'string' ? [paths] : paths;
    const id = pathsId(watchPaths);
    await invoke('plugin:fs-watch|unwatch', { id }).catch(() => {});
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
    const id = pathsId(watchPaths);
    await invoke('plugin:fs-watch|unwatch', { id }).catch(() => {});
    await invoke('plugin:fs-watch|watch', { id, paths: watchPaths, options: opts });
    const unlisten = await w.listen(`watcher://raw-event/${id}`, (event) => {
        cb(event.payload);
    });
    return () => {
        void invoke('plugin:fs-watch|unwatch', { id });
        unlisten();
    };
}
