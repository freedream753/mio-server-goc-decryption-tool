// 符文自动识别结果窗口（v1.1.0）
// 数据流：Rust 全屏识别 → emit `rune_scan` + 静态 RUNE_SCAN，挂载后主动 `get_rune_scan()` 拉一次防竞态；
//        payload = { ok, slots:[{rune,color}×4], source, msg }，rune 0=空 1=火 2=水 3=风 4=土 5=雷，color 0=红 1=黄 2=绿。
// 8 项全部非 null 才自动追加一条历史（与末条完全相同则跳过，同一屏重复按热键不重复记），
//       任一为 null 不写入，等用户在窗口里补齐后手点「记录本次」。
// 求解一律走 Web Worker（runes.worker.js + runes.js 的 createSolver），主线程不建 1296×1296 矩阵。
// 窗口为 NOACTIVATE：select 聚焦时必须 focus_result_window 才能进入键盘输入；
//       所有交互都 reset_result_timer，避免操作途中被 10 秒倒计时隐藏。
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
import { writeText } from '@tauri-apps/api/clipboard';
import React, { useEffect, useRef, useState } from 'react';
import { codes, symbols } from '../../utils/runes';
import {
    MAX_TURNS,
    SECRET_COUNT,
    SESSION_KEY,
    appendRecord,
    isRecorded,
    loadSession,
    recordFromSlots,
    remainingTurns,
    saveSession,
    startNewGame,
    undoLast,
} from '../../utils/rune-store';
import './style.css';

const COUNTDOWN = 10;
const COLORS = ['红', '黄', '绿'];
const emptySlots = () => Array.from({ length: 4 }, () => ({ rune: null, color: null }));
const label = (index) => codes[index].map((i) => symbols[i]).join(' · ');

// payload -> 4 个可编辑位：缺失/越界一律按未识别（null）处理
function normalizeSlots(slots) {
    const list = Array.isArray(slots) ? slots : [];
    return Array.from({ length: 4 }, (_, i) => {
        const rune = list[i]?.rune;
        const color = list[i]?.color;
        return {
            rune: Number.isInteger(rune) && rune >= 0 && rune < 6 ? rune : null,
            color: Number.isInteger(color) && color >= 0 && color < 3 ? color : null,
        };
    });
}

export default function Result() {
    const [slots, setSlots] = useState(emptySlots);
    const [notice, setNotice] = useState(''); // 识别失败原因（payload.msg）
    const [session, setSession] = useState(loadSession);
    const [result, setResult] = useState(null);
    const [busy, setBusy] = useState(true);
    const [error, setError] = useState('');
    const [copied, setCopied] = useState(false);
    const [count, setCount] = useState(COUNTDOWN);
    const countRef = useRef(COUNTDOWN);
    const pausedRef = useRef(false); // 拖动窗口期间冻结倒计时
    const graceTORef = useRef(null); // 松开拖动后 2s 恢复倒计时的定时器
    const copyTORef = useRef(null);
    const worker = useRef(null);
    const serial = useRef(0);

    const resetTimer = () => {
        countRef.current = COUNTDOWN;
        setCount(COUNTDOWN);
    };

    // 改动即重置倒计时：Rust 侧兜底隐藏同步推迟
    const touchInteract = () => {
        resetTimer();
        invoke('reset_result_timer').catch(() => {});
    };

    // 会话只经 rune-store 读写：写操作前一律重新加载，避免覆盖另一窗口刚写入的记录
    const commit = (next) => {
        setSession(next);
        saveSession(next);
    };

    const applyPayload = (payload) => {
        if (!payload) return;
        const next = normalizeSlots(payload.slots);
        setSlots(next);
        setNotice(payload.ok === false ? payload.msg || '识别失败，请重试或改用「框选识别」。' : '');
        setCopied(false);
        resetTimer();
        const base = loadSession();
        const record = recordFromSlots(next.map((s) => s.rune), next.map((s) => s.color));
        if (!record) {
            setSession(base); // 识别不完整：不写历史
            return;
        }
        const appended = appendRecord(base, record);
        if (appended.added) commit(appended.session);
        else setSession(base);
    };

    useEffect(() => {
        // 挂载后拉一次最近结果，避免 Rust emit 早于监听注册的竞态
        invoke('get_rune_scan')
            .then(applyPayload)
            .catch(() => {});
        const un = listen('rune_scan', (e) => applyPayload(e.payload));
        return () => {
            un.then((f) => f());
        };
    }, []);

    // 另一窗口（符文助手）改了同一份会话时刷新展示，但不回写，避免心跳式互相触发
    useEffect(() => {
        const onStorage = (e) => {
            if (!e.key || e.key === SESSION_KEY) setSession(loadSession());
        };
        window.addEventListener('storage', onStorage);
        return () => window.removeEventListener('storage', onStorage);
    }, []);

    // 求解：worker 内复用同一份 createSolver，主线程只收结果
    useEffect(() => {
        const w = new Worker(new URL('../../utils/runes.worker.js', import.meta.url), { type: 'module' });
        worker.current = w;
        w.onmessage = ({ data }) => {
            if (data.id !== serial.current) return;
            setBusy(false);
            setError(data.error || '');
            setResult(data.error ? null : data);
        };
        w.onerror = () => {
            setBusy(false);
            setError('策略计算失败，请关闭窗口后重新打开。');
        };
        return () => w.terminate();
    }, []);

    useEffect(() => {
        if (!worker.current) return;
        setBusy(true);
        setError('');
        setResult(null);
        worker.current.postMessage({ id: ++serial.current, mode: session.mode, history: session.history, spent: session.spent });
    }, [session]);

    // 倒计时到期只隐藏不销毁（下次复用；Rust 侧另有兜底隐藏）
    useEffect(() => {
        const timer = setInterval(() => {
            if (pausedRef.current) return; // 拖动中：时间不走
            countRef.current -= 1;
            if (countRef.current <= 0) {
                invoke('hide_result_window').catch(() => {});
                resetTimer();
            } else {
                setCount(countRef.current);
            }
        }, 1000);
        return () => clearInterval(timer);
    }, []);

    // 拖动结束（Rust 检测左键松开）：保持冻结 2s 后重新开始倒计时
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

    // 按下窗口即重置倒计时；按下头部（拖拽区）= 开始拖动
    const onWindowPress = (e) => {
        touchInteract();
        if (e.target.closest('.result-header')) {
            pausedRef.current = true;
            clearTimeout(graceTORef.current);
            invoke('begin_result_drag').catch(() => {});
        }
    };

    const closeResult = () => {
        pausedRef.current = false;
        clearTimeout(graceTORef.current);
        resetTimer();
        invoke('hide_result_window').catch(() => {});
    };

    // 框选识别：Rust 命令内先隐藏本窗口再拉起手动框选，是全屏定位失败时的降级入口
    const startManualOcr = () => {
        invoke('ocr_recognize').catch(() => {});
    };

    const setRune = (i, value) => {
        touchInteract();
        setSlots((list) => list.map((s, j) => (j === i ? { ...s, rune: value === '' ? null : Number(value) } : s)));
    };

    const setColor = (i, value) => {
        touchInteract();
        setSlots((list) => list.map((s, j) => (j === i ? { ...s, color: value } : s)));
    };

    const recordNow = () => {
        touchInteract();
        const record = recordFromSlots(slots.map((s) => s.rune), slots.map((s) => s.color));
        if (!record) return;
        const appended = appendRecord(loadSession(), record);
        if (appended.added) commit(appended.session);
    };

    const undoNow = () => {
        touchInteract();
        const base = loadSession();
        if (!base.history.length) return;
        commit(undoLast(base));
    };

    const newRound = () => {
        touchInteract();
        commit(startNewGame(loadSession()));
    };

    const copyGuess = async () => {
        if (result?.guess == null) return;
        try {
            await writeText(label(result.guess));
            setCopied(true);
            clearTimeout(copyTORef.current);
            copyTORef.current = setTimeout(() => setCopied(false), 1200);
        } catch {
            // 剪贴板失败不打断展示
        }
        touchInteract();
    };

    const record = recordFromSlots(slots.map((s) => s.rune), slots.map((s) => s.color));
    const recorded = record != null && isRecorded(session, record.guess, record.result);
    const left = remainingTurns(session);
    const pending = slots.filter((s) => s.rune == null || s.color == null).length;
    const canRecord = record != null && !recorded && left > 0;
    const status = busy
        ? '正在计算可保证的策略…'
        : error ||
          (result?.count === 0
              ? '反馈与当前规则或历史矛盾。请「撤回本次」检查，或到符文助手切换判色规则。'
              : result?.solved
                ? '解密成功！开始下一局前请点「新一局」。'
                : result?.turns === 0
                  ? '四次机会已用完，请点「新一局」。'
                  : result?.guess == null
                    ? '剩余次数内无法保证解出。可撤回检查，或继续录入下一次实际猜测筛选。'
                    : '可在游戏中提交推荐答案，提交后按 Alt+C 重新识别反馈。');
    const hint =
        record == null
            ? `识别不完整（${pending} 位待确认）：补齐 4 个符文与 4 个颜色后点「记录本次」。`
            : recorded
              ? '本次已记录（历史末条）。'
              : left <= 0
                ? '本局四次机会已用完，请点「新一局」。'
                : '本次未记录：确认无误后点「记录本次」。';

    return (
        <div className="result-container" data-tauri-drag-region onMouseDown={onWindowPress}>
            <div className="result-header" data-tauri-drag-region>
                {/* Tauri v1 拖拽区只识别 mousedown 的精确 target，header 子元素必须逐个带上属性 */}
                <span className="result-grip" data-tauri-drag-region title="拖动窗口">⠿</span>
                {/* 框选识别按钮：四角方框标志，按下先隐藏本窗口再拉起手动框选 */}
                <span
                    className="result-ocr"
                    title="框选识别：手动框选要识别的区域（全屏定位失败时使用）"
                    onMouseDown={(e) => e.stopPropagation()}
                    onClick={startManualOcr}
                >
                    <svg width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                        <path d="M1 4 V1 H4" />
                        <path d="M8 1 H11 V4" />
                        <path d="M11 8 V11 H8" />
                        <path d="M4 11 H1 V8" />
                    </svg>
                    <span className="result-ocr-text">框选</span>
                </span>
                <span className="result-title" data-tauri-drag-region>GOC · 符文自动识别</span>
                <span className="result-countdown" data-tauri-drag-region>{count}s</span>
                {/* 关闭按钮：不带拖拽区属性，阻止 mousedown 冒泡避免误启动拖动 */}
                <span className="result-close" title="关闭" onMouseDown={(e) => e.stopPropagation()} onClick={closeResult}>✕</span>
            </div>

            <div className="result-slots">
                {slots.map((slot, i) => {
                    const isPending = slot.rune == null || slot.color == null;
                    return (
                        <div className={`result-slot ${isPending ? 'is-pending' : ''}`} key={i}>
                            <span className="result-slot-index">第 {i + 1} 位</span>
                            <select
                                className="result-select"
                                aria-label={`第${i + 1}位符文`}
                                value={slot.rune == null ? '' : slot.rune}
                                onMouseDown={(e) => e.stopPropagation()}
                                // NOACTIVATE 窗口：不程序化激活就收不到键盘输入
                                onFocus={() => invoke('focus_result_window').catch(() => {})}
                                onChange={(e) => setRune(i, e.target.value)}
                            >
                                <option value="">待确认</option>
                                {[...symbols].map((s, n) => (
                                    <option value={n} key={s}>{s}</option>
                                ))}
                            </select>
                            <div className={`result-colors ${slot.color == null ? 'is-unset' : ''}`}>
                                {COLORS.map((c, n) => (
                                    <button
                                        type="button"
                                        key={c}
                                        className={`result-color c${n} ${slot.color === n ? 'selected' : ''}`}
                                        title={`第${i + 1}位${c}色`}
                                        aria-label={`第${i + 1}位${c}色`}
                                        aria-pressed={slot.color === n}
                                        onClick={() => setColor(i, n)}
                                    >{c}</button>
                                ))}
                            </div>
                        </div>
                    );
                })}
            </div>

            <section className="result-status" role="status" aria-live="polite">
                <div className="status-line">
                    <span>剩余机会 <b>{left}</b> / {MAX_TURNS}</span>
                    <span>候选 <b>{result?.count ?? '…'}</b> / {SECRET_COUNT}</span>
                    <span className="status-mode">{session.mode === 'count' ? '按数量匹配' : '存在即黄'}</span>
                </div>
                <p className={notice ? 'is-warn' : result?.solved ? 'is-solved' : ''}>
                    {notice ? `识别失败：${notice}` : status}
                </p>
                <div className="status-hint">{hint}</div>
            </section>

            <div
                className={`result-recommend ${result?.solved ? 'is-solved' : ''} ${result?.guess == null ? 'is-idle' : ''}`}
                title={result?.guess == null ? '暂无推荐提交' : '点击复制推荐提交'}
                onClick={copyGuess}
            >
                <span className="recommend-text">
                    {result?.solved
                        ? '解密成功'
                        : result?.guess != null
                          ? `推荐提交：${label(result.guess)}`
                          : '推荐提交：—'}
                </span>
                {result?.guess != null && <span className="recommend-copy">{copied ? '已复制' : '复制'}</span>}
            </div>

            <div className="result-actions">
                <button type="button" className="act-record" disabled={!canRecord} onClick={recordNow}>记录本次</button>
                <button type="button" disabled={!session.history.length} onClick={undoNow}>撤回本次</button>
                <button type="button" onClick={newRound}>新一局</button>
            </div>
        </div>
    );
}
