import { Store } from '../lib/store-api';
import { appConfigDir, join } from '@tauri-apps/api/path';
import { listen } from '@tauri-apps/api/event';

export let store = new Store();

let initialization;

// One subscription per WebView. The native file watcher belongs to the app,
// so destroying a window cannot leave native watchers or worker threads behind.
export function initStore() {
    initialization ??= initialize();
    return initialization;
}

async function initialize() {
    const appConfigDirPath = await appConfigDir();
    const appConfigPath = await join(appConfigDirPath, 'config.json');
    store = new Store(appConfigPath);
    const reload = async () => {
        try {
            await store.load();
        } catch (error) {
            // Keep cached/default settings usable if the file is temporarily
            // missing, locked or malformed. A later native event retries load.
            console.warn('Could not load configuration:', error);
        }
    };
    try {
        await listen('config_file_changed', reload);
    } catch (error) {
        console.warn('Could not subscribe to configuration changes:', error);
    }
    await reload();
}
