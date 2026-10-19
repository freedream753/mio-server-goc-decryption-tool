// 隐藏 OCR 解密 worker：
// Rust 端截图框选完成后创建本窗口（不显示），本组件自动执行：
// 截图 base64 -> Tesseract.js 英文识别 -> 检测英文 -> 凯撒自动破解 -> 明文写入剪贴板 -> 系统通知 -> 关闭窗口
import { appWindow } from '@tauri-apps/api/window';
import { writeText } from '@tauri-apps/api/clipboard';
import { sendNotification } from '@tauri-apps/api/notification';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api';
import React, { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import { useConfig } from '../../hooks';
import { crackCaesar, hasEnglishLetters } from '../../utils/caesar';
import { recognize } from '../../services/recognize/tesseract';

export default function Recognize() {
    const { t } = useTranslation();
    const [deleteNewline] = useConfig('recognize_delete_newline', false);
    const busyRef = useRef(false);

    const notify = async (title, body) => {
        try {
            await sendNotification({ title, body });
        } catch (e) {
            console.error(e);
        }
    };

    const run = async () => {
        if (busyRef.current) return;
        busyRef.current = true;
        try {
            const base64 = await invoke('get_base64');
            if (!base64) {
                await notify(t('recognize.decrypt_error'), t('recognize.no_image'));
                return;
            }
            let text = (await recognize(base64)).trim();
            if (deleteNewline) {
                text = text.replace(/\-\s+/g, '').replace(/\s+/g, ' ');
            }
            if (hasEnglishLetters(text)) {
                // 识别到英文：自动凯撒破解，明文进剪贴板并系统通知
                const { shift, plaintext } = crackCaesar(text);
                await writeText(plaintext);
                await notify(t('recognize.decrypt_success', { shift: String(shift) }), plaintext);
            } else {
                // 未识别到英文：原文直接进剪贴板并系统通知
                await writeText(text);
                await notify(t('recognize.copy_success'), text);
            }
        } catch (e) {
            await notify(t('recognize.decrypt_error'), e?.toString() ?? String(e));
        } finally {
            busyRef.current = false;
            await appWindow.close();
        }
    };

    useEffect(() => {
        if (deleteNewline === null) return;
        // 窗口刚创建时 Rust 端已 emit 过 new_image，挂载后主动跑一次兜底
        run();
        const unlisten = listen('new_image', () => {
            run();
        });
        return () => {
            unlisten.then((f) => {
                f();
            });
        };
    }, [deleteNewline]);

    return null;
}
