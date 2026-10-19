// tauri-plugin-store v1 JS API 本地实现（替代 github:tauri-apps/tauri-plugin-store#v1）
// 协议与官方一致：plugin:store|load / get / set / save ...
import { invoke } from '@tauri-apps/api/tauri';

export class Store {
    constructor(path) {
        this.path = path;
    }

    async set(key, value) {
        return await invoke('plugin:store|set', { path: this.path, key, value });
    }

    async get(key) {
        return await invoke('plugin:store|get', { path: this.path, key });
    }

    async has(key) {
        return await invoke('plugin:store|has', { path: this.path, key });
    }

    async delete(key) {
        return await invoke('plugin:store|delete', { path: this.path, key });
    }

    async clear() {
        return await invoke('plugin:store|clear', { path: this.path });
    }

    async reset() {
        return await invoke('plugin:store|reset', { path: this.path });
    }

    async keys() {
        return await invoke('plugin:store|keys', { path: this.path });
    }

    async values() {
        return await invoke('plugin:store|values', { path: this.path });
    }

    async entries() {
        return await invoke('plugin:store|entries', { path: this.path });
    }

    async length() {
        return await invoke('plugin:store|length', { path: this.path });
    }

    async load() {
        return await invoke('plugin:store|load', { path: this.path });
    }

    async save() {
        return await invoke('plugin:store|save', { path: this.path });
    }
}
