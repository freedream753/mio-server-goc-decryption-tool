// 解题结果窗口（v1.0.3-hotfix.2 分级输出管线）
// 特性：始终置顶不抢焦点（Rust 侧 WS_EX_NOACTIVATE + SW_SHOWNA），
//       点击答案行直接复制并重置 10 秒倒计时，到期自动隐藏；
//       分级交互兜底（Rust 侧已按 词表反查>标签计算 排序后下发）：
//         - manual：标签与词表都未命中 -> [-][+n][+] 方形按键手动位移，实时凯撒出答案
//         - input：密文未识别 -> 空密文输入框；已有密文双击可编辑（改完自动词表反查）
//       NOACTIVATE 窗口点击不激活：输入框聚焦时调用 focus_result_window 程序化激活；
//       自学习词库在设置页维护（Config 页），本窗口只做展示与复制。
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
import { writeText } from '@tauri-apps/api/clipboard';
import React, { useEffect, useRef, useState } from 'react';
import { caesarShift } from '../../utils/caesar';
import './style.css';

const COUNTDOWN = 10;

const statusIcon = {
    ok: '✓',
    learned: '★',
    table: '≡',
    unknown: '?',
};

// 位移量显示：+N / -N / ±（未选）
const fmtShift = (n) => (n > 0 ? `+${n}` : n < 0 ? `${n}` : '±');

export default function Result() {
    const [cards, setCards] = useState([]);
    const [copied, setCopied] = useState(-1);
    const [words, setWords] = useState(null); // 答案词表 Set（Rust 下发，前后端同源）
    // 交互编辑状态：index -> { cipher?: 手改密文, shift?: 手选位移 }
    const [edits, setEdits] = useState({});
    const [editing, setEditing] = useState(-1); // 正在编辑密文的行号
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
        setEdits({});
        setEditing(-1);
        setCopied(-1);
        resetTimer();
        // 每次新结果重取词表：设置页改动核心词库后立即生效
        invoke('get_answer_words')
            .then((w) => setWords(new Set(w)))
            .catch(() => {});
        if (payload.show) {
            invoke('show_result_window').catch(() => {});
        }
    };

    useEffect(() => {
        // 挂载后拉取最新结果（避免 Rust emit 早于监听注册的竞态）
        invoke('get_solve_result')
            .then(applyPayload)
            .catch(() => {});
        invoke('get_answer_words')
            .then((w) => setWords(new Set(w)))
            .catch(() => setWords(new Set()));
        const un = listen('solve_result', (e) => applyPayload(e.payload));
        return () => {
            un.then((f) => f());
        };
    }, []);

    // 倒计时到期自动隐藏（只隐藏不销毁，下次复用；Rust 侧另有兜底隐藏）
    // 注意：点击复制 / 按下拖动窗口 / 手动交互均会重置倒计时，鼠标划过页面不重置；
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

    // ===== 手动交互：改动即重置倒计时，防止输入中途窗口隐藏 =====
    const touchInteract = () => {
        resetTimer();
        invoke('reset_result_timer').catch(() => {});
    };

    // 词表反查：cipher 唯一决定 (标签, 答案)；未命中返回 null
    // 只扫 n=1..13：mod 26 下 后移N ≡ 前移(26-N)，n 升序首个命中即最短表示
    const lookupTable = (cipher) => {
        if (!words || cipher.length < 3) return null;
        for (let n = 1; n <= 13; n++) {
            for (const dir of [1, -1]) {
                const plain = caesarShift(cipher, dir * n);
                if (words.has(plain)) {
                    return { label: `${dir > 0 ? '前移' : '后移'}${n}位`, answer: plain };
                }
            }
        }
        return null;
    };

    // 行派生状态：无后端定案（manual/input）或用户改过密文时前端重算
    const deriveRow = (card, edit) => {
        const cipher = (edit.cipher ?? card.cipher ?? '').trim().toUpperCase();
        const out = { cipher, label: card.label || '', answer: card.answer || '', status: card.status };
        if (card.status !== 'manual' && card.status !== 'input' && edit.cipher == null) {
            return out; // 后端已定案（ok/learned/table/unknown）
        }
        out.label = '';
        out.answer = '';
        if (cipher.length < 3) {
            out.status = 'input';
            return out;
        }
        if (edit.shift !== undefined) {
            // 手动 +/-：位移直接凯撒，命中词表即视为已验证
            out.answer = caesarShift(cipher, edit.shift);
            out.status = words?.has(out.answer) ? 'ok' : 'unknown';
            return out;
        }
        const hit = lookupTable(cipher);
        if (hit) {
            out.label = hit.label;
            out.answer = hit.answer;
            out.status = 'table';
        } else {
            out.status = 'manual';
        }
        return out;
    };

    const bumpShift = (i, d) => {
        touchInteract();
        setEdits((m) => {
            let next = (m[i]?.shift ?? 0) + d;
            if (next === 0) next = d; // 跳过 0：从 -1 点 + 直接到 +1
            // 回绕到 ±13：mod 26 等价取最短表示（+14 → -12），避免显示 后移24位 之类误导值
            if (next > 13) next -= 26;
            if (next < -13) next += 26;
            return { ...m, [i]: { ...m[i], shift: next } };
        });
    };

    const onCipherEdit = (i, v) => {
        touchInteract();
        setEdits((m) => ({ ...m, [i]: { ...m[i], cipher: v.toUpperCase() } }));
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
                {cards.map((card, i) => {
                    const edit = edits[i] || {};
                    const row = deriveRow(card, edit);
                    const isEditing = editing === i;
                    // ± 按键仅待定案（manual）或用户已改密文时显示；
                    // learned/error 行后端 label 为空，此前会误渲染死按键（点击无效果）
                    const showShiftCtl = row.cipher.length >= 3
                        && (row.status === 'manual' || edit.cipher != null || edit.shift !== undefined);
                    return (
                        <div
                            key={i}
                            className={`result-row ${row.status === 'error' ? 'is-error' : ''}`}
                            onClick={() => copyAnswer(i, row.answer)}
                        >
                            {/* 密文位：空=输入框（优先级3），有值=双击编辑 */}
                            {isEditing || !row.cipher ? (
                                <input
                                    className="cipher-input"
                                    placeholder="输入密文"
                                    title="输入密文（A-Z），自动词表反查"
                                    value={edit.cipher ?? row.cipher}
                                    onClick={(e) => e.stopPropagation()}
                                    onFocus={() => invoke('focus_result_window').catch(() => {})}
                                    onChange={(e) => onCipherEdit(i, e.target.value)}
                                    onBlur={() => setEditing(-1)}
                                    onKeyDown={(e) => {
                                        if (e.key === 'Enter') setEditing(-1);
                                        if (e.key === 'Escape') setEditing(-1);
                                    }}
                                />
                            ) : (
                                <span
                                    className="row-cipher"
                                    title="双击编辑密文"
                                    onDoubleClick={() => setEditing(i)}
                                >{row.cipher}</span>
                            )}
                            {/* 位移位：待定案/编辑态显示 [-n+] 方形按键，已定案行显示标签徽章，learned/error 不渲染 */}
                            {row.label ? (
                                <span className="row-label">{row.label}</span>
                            ) : (
                                showShiftCtl && (
                                    <span className="shift-ctl" onClick={(e) => e.stopPropagation()}>
                                        <button
                                            className="shift-btn"
                                            title="每字母后移 1 位（-N）"
                                            onClick={() => bumpShift(i, -1)}
                                        >−</button>
                                        <span className={`shift-val ${edit.shift ? 'has' : ''}`}>{fmtShift(edit.shift ?? 0)}</span>
                                        <button
                                            className="shift-btn"
                                            title="每字母前移 1 位（+N）"
                                            onClick={() => bumpShift(i, 1)}
                                        >+</button>
                                    </span>
                                )
                            )}
                            <span className="row-arrow">→</span>
                            <span className={`row-answer st-${row.status}`}>
                                {row.answer
                                    || (row.status === 'input' ? '输入密文'
                                        : row.status === 'manual' ? '点±选位移'
                                        : card.msg || '--')}
                                {['ok', 'learned', 'table', 'unknown'].includes(row.status) && (
                                    <i className="row-status">{statusIcon[row.status] ?? '?'}</i>
                                )}
                            </span>
                            {copied === i && <span className="row-copied">已复制</span>}
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
