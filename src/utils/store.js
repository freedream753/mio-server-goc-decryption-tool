import { Store } from '../lib/store-api';
import { appConfigDir, join } from '@tauri-apps/api/path';
import { watch } from '../lib/fs-watch-api';
import { invoke } from '@tauri-apps/api';

export let store = new Store();

export async function initStore() {
    const appConfigDirPath = await appConfigDir();
    const appConfigPath = await join(appConfigDirPath, 'config.json');
    store = new Store(appConfigPath);
    // 文件监听是增强功能：config.json 运行期间被移走等原因导致注册失败时不能
    // 阻断窗口初始化（main.jsx 仅在 initStore 成功后渲染页面，此前失败 = 永久白屏）
    try {
        await watch(appConfigPath, async () => {
            await store.load();
            await invoke('reload_store');
        });
    } catch (e) {
        console.warn('config.json 监听注册失败，已跳过（不影响窗口使用）:', e);
    }
}
