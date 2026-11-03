// 隐藏 OCR 解密 worker：
// Rust 端截图框选完成后创建本窗口（不显示），本组件自动执行：
// 截图 base64 -> Tesseract.js 英文识别 -> 检测英文 -> 凯撒自动破解 -> 明文写入剪贴板 -> 系统通知 -> 关闭窗口
// 各失败分支均写入后台日志文件，供用户通过托盘「查看日志」带文件反馈
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

// 系统通知标题过长会被系统截断，超长时手动截断加省略号
const NOTIFY_TITLE_MAX = 64;
const truncateTitle = (s) => (s.length > NOTIFY_TITLE_MAX ? `${s.slice(0, NOTIFY_TITLE_MAX)}…` : s);

// 错误对象转字符串（Tauri invoke 报错通常是字符串）
const errText = (e) => e?.toString() ?? String(e);

// 前端失败事件转发到 Rust 日志文件（托盘「查看日志」可打开），转发失败静默忽略
const logEvent = (level, message) => {
    invoke('log_event', { level, message }).catch(() => {});
};

export default function Recognize() {
    const { t } = useTranslation();
    const [deleteNewline] = useConfig('recognize_delete_newline', false);
    const busyRef = useRef(false);

    const notify = async (title, body) => {
        try {
            await sendNotification({ title, body });
        } catch (e) {
            // 通知发送失败用户无感知，日志是唯一排查线索
            logEvent('warn', `[notify] 系统通知发送失败: ${errText(e)}`);
        }
    };

    // 写剪贴板：失败时记 error 日志并弹「复制到剪贴板失败」，返回 false 供调用方终止后续成功通知
    const copyText = async (text) => {
        try {
            await writeText(text);
            return true;
        } catch (e) {
            logEvent('error', `[clipboard] 剪贴板写入失败（可能被其他程序占用或权限不足）: ${errText(e)}`);
            await notify(t('recognize.copy_failed'), errText(e));
            return false;
        }
    };

    const run = async () => {
        if (busyRef.current) return;
        busyRef.current = true;
        // 记录当前失败阶段，兜底 catch 写日志时定位用
        let stage = 'get_screenshot';
        try {
            const base64 = await invoke('get_base64');
            if (!base64) {
                logEvent('warn', '[no_image] get_base64 返回空（截图缓存文件缺失或读取失败）');
                await notify(t('recognize.decrypt_error'), t('recognize.no_image'));
                return;
            }
            stage = 'ocr';
            let text = (await recognize(base64)).trim();
            if (deleteNewline) {
                text = text.replace(/\-\s+/g, '').replace(/\s+/g, ' ');
            }
            if (hasEnglishLetters(text)) {
                // 识别到英文：自动凯撒破解，明文进剪贴板，系统通知标题=OCR识别：原文、正文=明文
                const { plaintext } = crackCaesar(text);
                if (await copyText(plaintext)) {
                    await notify(truncateTitle(`OCR识别：${text}`), plaintext);
                }
            } else {
                // 未识别到英文：原文直接进剪贴板并系统通知
                if (await copyText(text)) {
                    await notify(t('recognize.copy_success'), text);
                }
            }
        } catch (e) {
            logEvent('error', `[${stage}] 处理失败: ${errText(e)}`);
            await notify(t('recognize.decrypt_error'), errText(e));
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
