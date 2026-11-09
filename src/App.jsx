import { appWindow } from '@tauri-apps/api/window';
import { BrowserRouter } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
const warn = console.warn;
import React, { useEffect } from 'react';
import { useTheme } from 'next-themes';

import Screenshot from './window/Screenshot';
import Result from './window/Result';
import Runes from './window/Runes';
import { store } from './utils/store';
import Config from './window/Config';
import { useConfig } from './hooks';
import './style.css';
import './i18n';

const windowMap = {
    screenshot: <Screenshot />,
    config: <Config />,
    result: <Result />,
    runes: <Runes />,
};

export default function App() {
    const [appTheme] = useConfig('app_theme', 'system');
    const [appLanguage] = useConfig('app_language', 'en');
    const [appFont] = useConfig('app_font', 'default');
    const [appFallbackFont] = useConfig('app_fallback_font', 'default');
    const [appFontSize] = useConfig('app_font_size', 16);
    const { setTheme } = useTheme();
    const { i18n } = useTranslation();

    useEffect(() => {
        store.load();
    }, []);

    useEffect(() => {
        // 拦截快捷键并清理监听器，避免重复挂载后监听器泄漏
        const handler = async (e) => {
            let allowKeys = ['c', 'v', 'x', 'a', 'z', 'y'];
            if (e.ctrlKey && !allowKeys.includes(e.key.toLowerCase())) {
                e.preventDefault();
            }
            if (e.key.startsWith('F') && e.key.length > 1) {
                e.preventDefault();
            }
            if (e.key === 'Escape') {
                if (appWindow.label === 'runes') {
                    await appWindow.hide();
                    return;
                }
                // 结果窗口只隐藏不销毁（复用 + Rust 兜底隐藏），Escape 关闭会把
                // 整窗销毁导致三卡全部消失（密文编辑态按 Esc 取消编辑即触发）
                if (appWindow.label !== 'result') {
                    await appWindow.close();
                }
            }
        };
        document.addEventListener('keydown', handler);
        return () => {
            document.removeEventListener('keydown', handler);
        };
    }, []);

    useEffect(() => {
        if (appTheme !== null) {
            if (appTheme !== 'system') {
                setTheme(appTheme);
            } else {
                try {
                    if (window.matchMedia('(prefers-color-scheme: dark)').matches) {
                        setTheme('dark');
                    } else {
                        setTheme('light');
                    }
                    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => {
                        if (e.matches) {
                            setTheme('dark');
                        } else {
                            setTheme('light');
                        }
                    });
                } catch {
                    warn("Can't detect system theme.");
                }
            }
        }
    }, [appTheme]);

    useEffect(() => {
        if (appLanguage !== null) {
            i18n.changeLanguage(appLanguage);
        }
    }, [appLanguage]);

    useEffect(() => {
        if (appFont !== null && appFallbackFont !== null) {
            document.documentElement.style.fontFamily = `"${appFont === 'default' ? 'sans-serif' : appFont}","${
                appFallbackFont === 'default' ? 'sans-serif' : appFallbackFont
            }"`;
        }
        if (appFontSize !== null) {
            document.documentElement.style.fontSize = `${appFontSize}px`;
        }
    }, [appFont, appFallbackFont, appFontSize]);

    return <BrowserRouter>{windowMap[appWindow.label]}</BrowserRouter>;
}
