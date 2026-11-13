// 三关卡页面（v1.2.4：转轮页删除；v1.2.1 集成进热键结果窗口）：
// 密码破解 / 波形校准 / 西蒙符印。
// 样式沿用结果窗口紧凑暗色风格（lvl- 前缀，见 style.css）。
// 三个页面在 Result 里常驻挂载（隐藏仅 display:none），保证西蒙录制的
// 事件监听不随页签切换丢失，录制状态得以保留。
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
import { codes, symbols } from '../../utils/runes';
import {
    appendRecord,
    loadSession,
    saveSession,
    SESSION_EVENT,
    startNewGame,
    undoLast,
} from '../../utils/rune-store';

const SYMBOL_LIST = [...symbols];
const COLOR_NAMES = ['红', '黄', '绿'];
// 第三关反馈色循环节奏：null → 绿(2) → 红(0) → 黄(1) → null
const COLOR_CYCLE = [2, 0, 1];
const label = (index) => codes[index].map((i) => symbols[i]).join(' · ');

// ---------------- 第一关 波形校准 ----------------

// 波形预览曲线（与游戏画布目标线同构，不简化）：
// 优先用 Rust 拟合出的完整双正弦参数 result.curve
//（y = off + a1·sin(2πf1·x+p1) + a2·sin(2πf2·x+p2)，即识别到的目标线本身）；
// 旧缓存无 curve 时按四滑块逆映射退化画基波。
// 采样 240 点输出 SVG path（viewBox 100×100，y 翻转：值大靠上与游戏画布一致）。
// y 按采样点 min/max 动态归一化到 viewBox 的 8%~92%（单一线性映射，波形形状
// 不变形）：高振幅/双正弦叠加也不会超出预览框被上下裁切；常数线退化居中
function waveCurvePath(result) {
    const c = result?.curve;
    const s = result?.sliders;
    let a1; let f1; let p1; let o; let a2 = 0; let f2 = 0; let p2 = 0;
    if (c) {
        ({ a1, f1, p1, off: o, a2, f2, p2 } = c);
    } else if (s) {
        a1 = 0.2 + (s.amp / 100) * 0.4;
        f1 = 0.3 + (s.freq / 100) * 2.7;
        p1 = (s.phase / 100) * Math.PI * 2;
        o = 0.15 - (s.off / 100) * 0.36;
    } else {
        return '';
    }
    const TOP = 8;
    const BOTTOM = 92;
    const raw = [];
    let yMin = Infinity;
    let yMax = -Infinity;
    for (let i = 0; i <= 240; i++) {
        const x = i / 240;
        const y = o
            + a1 * Math.sin(2 * Math.PI * f1 * x + p1)
            + a2 * Math.sin(2 * Math.PI * f2 * x + p2);
        raw.push(y);
        if (y < yMin) yMin = y;
        if (y > yMax) yMax = y;
    }
    const span = yMax - yMin;
    return `M${raw.map((y, i) => {
        const px = ((i / 240) * 100).toFixed(2);
        const py = span > 1e-6 ? TOP + ((yMax - y) / span) * (BOTTOM - TOP) : (TOP + BOTTOM) / 2;
        return `${px},${py.toFixed(2)}`;
    }).join('L')}`;
}

export function WavePage() {
    const [result, setResult] = useState(null);

    useEffect(() => {
        // 挂载后拉最近结果（防 emit 早于监听注册的竞态），再挂监听
        invoke('get_wave_scan').then(setResult).catch(() => {});
        const un = listen('wave_scan', (e) => setResult(e.payload));
        return () => {
            un.then((f) => f());
        };
    }, []);

    const scan = () => invoke('wave_scan_cmd').catch(() => {});
    // 手动框选降级：隐藏本窗口 → 全屏框选波形画布 → 对选区跑同一求解管线
    const selectRegion = () => invoke('wave_select_region').catch(() => {});
    const s = result?.sliders;
    return (
        <div className="lvl-body">
            <p className="lvl-desc">
                按 Alt+X 或点「识别」反推四个滑块位置，照抄即可；画布被遮挡识别失败时，点「框选」手动圈住波形画布。开启标题旁「裁切模式」后只识别框内画面。
            </p>
            <div className="lvl-row">
                <button type="button" className="lvl-btn primary" onClick={scan}>识别</button>
                <button type="button" className="lvl-btn" onClick={selectRegion}>框选</button>
                {result && !result.ok && <span className="lvl-error">{result.msg}</span>}
            </div>
            {result?.ok && s && (
                <div className="wave-result">
                    {/* 反推曲线预览：完整双正弦（拟合自游戏目标线），照抄前可先目检 */}
                    <div className="wave-preview" title="识别到的波形画布目标线（与游戏内完全同构）">
                        <svg viewBox="0 0 100 100" preserveAspectRatio="none">
                            <path d={waveCurvePath(result)} />
                        </svg>
                    </div>
                    {/* 滑块可视化：与游戏四个滑块同构，滑钮位置即应照抄的位置 */}
                    {[
                        ['振幅', s.amp],
                        ['频率', s.freq],
                        ['相位', s.phase],
                        ['偏移', s.off],
                    ].map(([name, val]) => (
                        <div className="wave-slider" key={name}>
                            <span className="wave-slider-name">{name}</span>
                            <div className="wave-slider-track">
                                <div className="wave-slider-fill" style={{ width: `${val}%` }} />
                                <div className="wave-slider-thumb" style={{ left: `${val}%` }} />
                            </div>
                            <b className="wave-slider-val">{val}</b>
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
}

// ---------------- 第二关 西蒙符印 ----------------

export function SimonPage() {
    const [steps, setSteps] = useState([]);
    const [running, setRunning] = useState(false);
    const [error, setError] = useState('');
    const [info, setInfo] = useState('');
    const [target, setTarget] = useState(0); // 0=自由 4/5/6=关卡

    useEffect(() => {
        invoke('get_simon_steps').then(setSteps).catch(() => {});
        const unStep = listen('simon_step', (e) => {
            setSteps((list) => (list.some((s) => s.seq === e.payload.seq) ? list : [...list, e.payload]));
        });
        const unState = listen('simon_state', (e) => {
            setRunning(e.payload.running);
            setError(e.payload.error || '');
            setInfo(e.payload.info || '');
            if (!e.payload.running) {
                // 停止时以 Rust 侧 total 为准补齐（停止事件可能早于最后一帧的 step）
                invoke('get_simon_steps').then(setSteps).catch(() => {});
            }
        });
        return () => {
            unStep.then((f) => f());
            unState.then((f) => f());
        };
    }, []);

    const stop = () => invoke('simon_stop').catch(() => {});
    // 关卡按钮：第一/二/三关固定 4/5/6 步，录满自动暂停；自由 = 不限步数手动停
    const start = (t) => {
        setSteps([]);
        setError('');
        setInfo('');
        setTarget(t);
        invoke('simon_start', { target: t }).catch(() => {});
    };

    const LEVELS = [
        ['第一关', 4],
        ['第二关', 5],
        ['第三关', 6],
    ];

    return (
        <div className="lvl-body">
            <p className="lvl-desc">点关卡按钮（或按 Alt+C）开始录制，符号依次亮起会被自动记录；录满 4/5/6 步自动暂停，照顺序点击游戏符号。「自由」不限步数，再按一次停止。</p>
            <div className="lvl-row">
                {running ? (
                    <>
                        <button type="button" className="lvl-btn primary is-danger" onClick={stop}>停止录制</button>
                        <span className="lvl-hint">录制中 {steps.length}{target > 0 ? `/${target}` : ''}…</span>
                    </>
                ) : (
                    <>
                        {LEVELS.map(([name, n]) => (
                            <button type="button" key={n} className="lvl-btn" onClick={() => start(n)}>{name}</button>
                        ))}
                        <button type="button" className="lvl-btn primary" onClick={() => start(0)}>自由</button>
                    </>
                )}
                {error && <span className="lvl-error">{error}</span>}
                {!error && info && <span className="lvl-hint">{info}</span>}
            </div>
            {!!steps.length && (
                <div className="simon-steps">
                    {steps.map((s) => <span className="simon-step" key={s.seq}>{s.seq}. {s.name}</span>)}
                </div>
            )}
            {!steps.length && !running && <p className="lvl-hint">暂无记录。</p>}
        </div>
    );
}

// ---------------- 第三关 密码破解 ----------------

export function CipherPage() {
    const [session, setSession] = useState(loadSession);
    // 会话只经 rune-store 读写；写操作前重新加载，避免覆盖自动识别刚写入的记录。
    // 注意必须返回计算结果本身（setSession 无返回值，表达式体箭头函数会返回 undefined）
    const mutate = (fn) => {
        const next = fn(loadSession());
        setSession(next);
        return next;
    };
    const { history } = session;
    const [guess, setGuess] = useState([0, 1, 2, 3]);
    const [colors, setColors] = useState([null, null, null, null]);
    const [selected, setSelected] = useState(0);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    const worker = useRef(null);
    const serial = useRef(0);
    const sessionRef = useRef(session); // 当前会话序列化快照，广播去重用

    useEffect(() => {
        const w = new Worker(new URL('../../utils/runes.worker.js', import.meta.url), { type: 'module' });
        worker.current = w;
        w.onmessage = ({ data }) => {
            if (data.id !== serial.current) return;
            setBusy(false);
            setError(data.error || '');
            // 给出新的最优猜测，清空反馈方块等待用户录入
            if (!data.error && data.guess != null) setGuess([...codes[data.guess]]);
            setColors([null, null, null, null]);
            setSelected(0);
        };
        w.onerror = () => {
            setBusy(false);
            setError('策略计算失败，请关闭窗口后重新打开。');
        };
        // 挂载时若已有历史（自动识别或上一窗口先记录过），重算推荐而不是停留在初始猜测
        if (session.history.length) {
            setBusy(true);
            w.postMessage({ id: ++serial.current, history: session.history });
        }
        return () => w.terminate();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // 写入 localStorage 并广播 SESSION_EVENT（其他订阅方据此刷新）
    useEffect(() => {
        sessionRef.current = session;
        saveSession(session);
    }, [session]);

    // 自动识别（index.jsx）写入/其他窗口新一局时同步本页并重算推荐（storage 事件不发给同文档写入方）
    useEffect(() => {
        const onChanged = () => {
            const next = loadSession();
            if (JSON.stringify(next) === JSON.stringify(sessionRef.current)) return;
            sessionRef.current = next;
            setSession(next);
            if (worker.current) {
                setBusy(true);
                worker.current.postMessage({ id: ++serial.current, history: next.history });
            }
        };
        window.addEventListener(SESSION_EVENT, onChanged);
        return () => window.removeEventListener(SESSION_EVENT, onChanged);
    }, []);

    // 点反馈方块：循环 null → 绿 → 红 → 黄 → null，同时设为选中位
    const cycleColor = (i) => {
        setSelected(i);
        setColors((f) => f.map((v, j) => (j === i ? COLOR_CYCLE[(COLOR_CYCLE.indexOf(v) + 1) % COLOR_CYCLE.length] : v)));
    };

    // 点符文选项：改选中框符号并前进到下一框
    const pickRune = (n) => {
        setGuess((g) => g.map((v, j) => (j === selected ? n : v)));
        setSelected((s) => (s + 1) % 4);
    };

    // 修正：反馈齐全才可点 → 写入历史 → worker 重算新推荐
    const applyFix = () => {
        if (busy || colors.includes(null)) return;
        const encoded = guess.reduce((n, v) => n * 6 + v, 0);
        const encodedColors = colors.reduce((n, v, i) => n + v * 3 ** i, 0);
        const { session: next, added } = appendRecord(loadSession(), { guess: encoded, result: encodedColors });
        if (!added) return;
        setSession(next); // session 效果会落盘并广播 SESSION_EVENT，保持多窗口会话一致
        setBusy(true);
        setError('');
        worker.current.postMessage({ id: ++serial.current, history: next.history });
        // 四个反馈全绿 = 本局已破解（v1.2.7）：短暂停留让用户看到全绿，然后自动重开新一局
        if (encodedColors === 80) {
            window.setTimeout(() => newGame(), 600);
        }
    };

    const newGame = () => {
        setGuess([0, 1, 2, 3]);
        setColors([null, null, null, null]);
        setSelected(0);
        mutate(startNewGame);
        setBusy(true);
        worker.current?.postMessage({ id: ++serial.current, history: [] });
    };

    const undo = () => {
        if (!history.length) return;
        mutate(undoLast);
        setBusy(true);
        worker.current.postMessage({ id: ++serial.current, history: history.slice(0, -1) });
    };

    const turn = 1 + history.length; // 第几次猜测
    const ended = history.length >= 4;

    return (
        <div className="lvl-body">
            <div className="lvl-row between">
                <span className="lvl-status">{busy ? '正在计算下一步…' : error ||
                    (ended ? '四次已用完，请点「新一局」。' : `第 ${turn} 次猜测（照抄进游戏）：`)}</span>
                <span className="lvl-tools">
                    {/* 识别：与热键 Alt+V 同入口，全屏识别结果自动写入本页并重算推荐 */}
                    <button type="button" className="lvl-btn" onClick={() => invoke('rune_recognize').catch(() => {})}>识别</button>
                    <button type="button" className="lvl-btn" onClick={newGame}>新一局</button>
                    <button type="button" className="lvl-btn" disabled={!history.length} onClick={undo}>撤回</button>
                </span>
            </div>
            <p className="lvl-desc">点方块循环切换 <b className="lvl-c2">绿</b>/<b className="lvl-c0">红</b>/<b className="lvl-c1">黄</b>，切好后点「修正」。</p>
            <div className="cipher-slots">
                {guess.map((value, i) => (
                    <button
                        type="button"
                        key={i}
                        className={`cipher-slot ${selected === i ? 'is-selected' : ''} ${colors[i] != null ? `is-${colors[i]}` : ''}`}
                        onClick={() => cycleColor(i)}
                        title={colors[i] == null ? '点击标记反馈色：绿→红→黄循环' : `当前 ${COLOR_NAMES[colors[i]]}，点击切换`}
                    >
                        <span className="cipher-symbol">{symbols[value]}</span>
                        <span className="cipher-color-name">{colors[i] == null ? '标色' : COLOR_NAMES[colors[i]]}</span>
                    </button>
                ))}
            </div>
            <div className="cipher-options">
                {SYMBOL_LIST.map((s, n) => <button type="button" key={s} onClick={() => pickRune(n)} disabled={ended}>{s}</button>)}
            </div>
            <button
                type="button"
                className="lvl-btn primary"
                disabled={busy || ended || colors.includes(null)}
                onClick={applyFix}
            >修正</button>
            {!!history.length && (
                <ol className="rune-history">
                    {history.map((h, i) => (
                        <li key={i}>
                            <span>{i + 1}. {label(h.guess)}</span>
                            <span>
                                {[0, 1, 2, 3].map((j) => (
                                    <b key={j} className={`lvl-c${Math.floor(h.result / 3 ** j) % 3}`}>
                                        {COLOR_NAMES[Math.floor(h.result / 3 ** j) % 3]}
                                    </b>
                                ))}
                            </span>
                        </li>
                    ))}
                </ol>
            )}
        </div>
    );
}
