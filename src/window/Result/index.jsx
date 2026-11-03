// 解题结果窗口（v1.0.3 自动管线）
// 特性：始终置顶不抢焦点（Rust 侧 WS_EX_NOACTIVATE + SW_SHOWNA），
//       点击答案行直接复制并重置 10 秒倒计时，到期自动隐藏；
//       自学习词库在设置页维护（Config 页），本窗口只做展示与复制。
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
import { appWindow } from '@tauri-apps/api/window';
import { writeText } from '@tauri-apps/api/clipboard';
import React, { useEffect, useRef, useState } from 'react';
import './style.css';

const COUNTDOWN = 10;

const statusIcon = {
    ok: '✓',
    learned: '★',
    unknown: '?',
    error: '!',
};

export default function Result() {
    const [cards, setCards] = useState([]);
    const [copied, setCopied] = useState(-1);
    const [count, setCount] = useState(COUNTDOWN);
    const countRef = useRef(COUNTDOWN);
    const pausedRef = useRef(false); // 拖动窗口期间冻结倒计时
    const graceTORef = useRef(null); // 松开拖动后 2s 恢复倒计时的定时器

    const resetTimer = () => {
        countRef.current = COUNTDOWN;
        setCount(COUNTDOWN);
    };

    const applyPayload = async (payload) => {
        if (!payload) return;
        setCards(payload.cards ?? []);
        setCopied(-1);
        resetTimer();
        if (payload.show) {
            invoke('show_result_window').catch(() => {});
        }
    };

    useEffect(() => {
        // 挂载后拉取最新结果（避免 Rust emit 早于监听注册的竞态）
        invoke('get_solve_result')
            .then(applyPayload)
            .catch(() => {});
        const un = listen('solve_result', (e) => applyPayload(e.payload));
        return () => {
            un.then((f) => f());
        };
    }, []);

    // 倒计时到期自动隐藏（只隐藏不销毁，下次复用；Rust 侧另有兜底隐藏）
    // 注意：点击复制 / 按下拖动窗口会重置倒计时，鼠标划过页面不重置；
    //       拖动窗口期间倒计时冻结（pausedRef 由 Rust Moved 事件驱动）
    useEffect(() => {
        const timer = setInterval(() => {
            if (pausedRef.current) return; // 拖动中：时间不走
            countRef.current -= 1;
            if (countRef.current <= 0) {
                // 走 Rust SW_HIDE 隐藏（appWindow.hide() 因 tao 可见状态为 false 是 no-op）
                invoke('hide_result_window').catch(() => {});
                resetTimer();
            } else {
                setCount(countRef.current);
            }
        }, 1000);
        return () => clearInterval(timer);
    }, []);

    const copyAnswer = async (index, text) => {
        if (!text) return;
        try {
            await writeText(text);
            setCopied(index);
            resetTimer();
            // 同步推迟 Rust 侧兜底隐藏
            invoke('reset_result_timer').catch(() => {});
        } catch {
            // 忽略剪贴板失败，不打断展示
        }
    };

    // 按下窗口即重置倒计时；鼠标划过不会触发 mousedown，不受影响。
    // 按下头部（拖拽区）= 开始拖动：倒计时重置为 10s 并保持不动（pausedRef 冻结显示），
    // 左键松开后由 Rust 轮询检测并发 result-drag-end，前端等 2s 再恢复倒计时。
    const onWindowPress = (e) => {
        resetTimer();
        invoke('reset_result_timer').catch(() => {});
        if (e.target.closest('.result-header')) {
            pausedRef.current = true;
            clearTimeout(graceTORef.current);
            invoke('begin_result_drag').catch(() => {});
        }
    };

    // 拖动结束（Rust 检测到左键松开）：保持冻结 2s 后重新开始 10s 倒计时
    useEffect(() => {
        const un = listen('result-drag-end', () => {
            clearTimeout(graceTORef.current);
            graceTORef.current = setTimeout(() => {
                pausedRef.current = false;
                resetTimer();
            }, 2000);
        });
        return () => {
            un.then((f) => f());
        };
    }, []);

    // 关闭按钮：立即隐藏（走 Rust SW_HIDE，绕开 tao 可见状态 no-op 问题）
    const closeResult = () => {
        pausedRef.current = false;
        clearTimeout(graceTORef.current);
        resetTimer();
        invoke('hide_result_window').catch(() => {});
    };

    return (
        <div className="result-container" data-tauri-drag-region onMouseDown={onWindowPress}>
            <div className="result-header" data-tauri-drag-region>
                {/* Tauri v1 拖拽区只识别 mousedown 的精确 target，header 子元素必须逐个带上属性 */}
                <span className="result-grip" data-tauri-drag-region title="拖动窗口">⠿</span>
                <span className="result-title" data-tauri-drag-region>GOC解题妙妙小工具</span>
                <span className="result-countdown" data-tauri-drag-region>{count}s</span>
                {/* 关闭按钮：不带拖拽区属性，阻止 mousedown 冒泡避免误启动拖动/重置 */}
                <span
                    className="result-close"
                    title="关闭"
                    onMouseDown={(e) => e.stopPropagation()}
                    onClick={closeResult}
                >✕</span>
            </div>
            <div className="result-rows">
                {cards.length === 0 && <div className="result-empty">无识别结果</div>}
                {cards.map((card, i) => (
                    <div
                        key={i}
                        className={`result-row ${card.status === 'error' ? 'is-error' : ''}`}
                        onClick={() => copyAnswer(i, card.answer)}
                    >
                        <span className="row-cipher">{card.cipher || '--'}</span>
                        <span className="row-label">{card.label || ''}</span>
                        <span className="row-arrow">→</span>
                        <span className={`row-answer st-${card.status}`}>
                            {card.answer || card.msg || '--'}
                            {card.status !== 'error' && (
                                <i className="row-status">{statusIcon[card.status] ?? '?'}</i>
                            )}
                        </span>
                        {copied === i && <span className="row-copied">已复制</span>}
                    </div>
                ))}
            </div>
        </div>
    );
}
