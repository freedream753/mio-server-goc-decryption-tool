// 两关卡页面（noocr 支线：不使用截图/OCR，第二关西蒙页已移除）：
// 波形校准（空白画布手绘）/ 密码破解。
// 样式沿用结果窗口紧凑暗色风格（lvl- 前缀，见 style.css）。
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api';
import { Trans, useTranslation } from 'react-i18next';
import { codes, runeKeys } from '../../utils/runes';
import {
    appendRecord,
    loadSession,
    saveSession,
    SESSION_EVENT,
    startNewGame,
    undoLast,
} from '../../utils/rune-store';

// 反馈色索引（与 payload 约定一致 0=红 1=黄 2=绿）→ i18n key 后缀，显示时经 result.color.* 取本地化名
const COLOR_KEYS = ['red', 'yellow', 'green'];
// 第三关反馈色循环节奏（v1.2.9 默认全绿，无空档）：绿(2) → 红(0) → 黄(1) → 绿
const COLOR_CYCLE = [2, 0, 1];

// ---------------- 第一关 波形校准 ----------------

// 波形预览曲线（与游戏画布目标线同构，不简化）：
// 优先用 Rust 拟合出的完整双正弦参数 result.curve
//（y = off + a1·sin(2πf1·x+p1) + a2·sin(2πf2·x+p2)，即拟合出的目标线本身）；
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

// 手绘采集画布网格线（每 10 单位一条）：浅色低透明度，仅作绘制参考（样式见 style.css）
const CANVAS_GRID = [];
for (let i = 10; i < 100; i += 10) {
    CANVAS_GRID.push(<line key={`v${i}`} x1={i} y1="0" x2={i} y2="100" />);
    CANVAS_GRID.push(<line key={`h${i}`} x1="0" y1={i} x2="100" y2={i} />);
}

// 触发拟合所需的最少采样点数（低于此值曲线信息不足，拟合无意义）
const MIN_POINTS = 8;

export function WavePage() {
    const { t } = useTranslation();
    // 归一化手绘点（x∈[0,1]，y∈[-1,1] 向上为正，与后端 wave_fit_points 约定一致）
    const [points, setPoints] = useState([]);
    const [drawing, setDrawing] = useState(false);
    const [result, setResult] = useState(null);
    // points 的镜像 ref：指针事件流里读最新点集，不依赖渲染闭包时序
    const pointsRef = useRef([]);

    // 指针坐标 → viewBox 0~100 → 归一化（y 轴翻转：画布顶部为 +1，向上为正）
    const toNorm = (e) => {
        const rect = e.currentTarget.getBoundingClientRect();
        const px100 = ((e.clientX - rect.left) / rect.width) * 100;
        const py100 = ((e.clientY - rect.top) / rect.height) * 100;
        return [px100 / 100, -(py100 / 100 - 0.5) * 2];
    };

    // 调后端拟合：入参归一化点集，出参 { ok, msg, sliders, curve }
    const fit = (pts) => {
        if (pts.length < MIN_POINTS) return;
        invoke('wave_fit_points', { points: pts })
            .then(setResult)
            .catch(() => {});
    };

    const onPointerDown = (e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        pointsRef.current = [];
        setPoints([]);
        setResult(null);
        setDrawing(true);
    };

    const onPointerMove = (e) => {
        if (!drawing) return;
        const [x, y] = toNorm(e);
        const prev = pointsRef.current;
        const last = prev[prev.length - 1];
        // 与上一点 x 间距过小的点不采集：压缩数据量也避免折线抖动
        if (last && Math.abs(x - last[0]) <= 0.004) return;
        const next = [...prev, [x, y]];
        pointsRef.current = next;
        setPoints(next);
    };

    const onPointerUp = () => {
        setDrawing(false);
        fit(pointsRef.current);
    };

    const clear = () => {
        pointsRef.current = [];
        setPoints([]);
        setResult(null);
        setDrawing(false);
    };

    const s = result?.sliders;
    // 手绘折线（画布 viewBox 0~100 坐标，y 轴翻转回画布方向）
    const trace = points
        .map(([x, y]) => `${(x * 100).toFixed(2)},${((0.5 - y / 2) * 100).toFixed(2)}`)
        .join(' ');

    return (
        <div className="lvl-body">
            <p className="lvl-desc">{t('result.wave.desc')}</p>
            <div className="wave-canvas-wrap">
                <svg
                    className="wave-canvas"
                    viewBox="0 0 100 100"
                    preserveAspectRatio="none"
                    onPointerDown={onPointerDown}
                    onPointerMove={onPointerMove}
                    onPointerUp={onPointerUp}
                >
                    {CANVAS_GRID}
                    {!!points.length && <polyline className="wave-canvas-trace" points={trace} />}
                    {result?.ok && <path className="wave-canvas-fit" d={waveCurvePath(result)} />}
                </svg>
                {/* 空态提示：纯展示层，pointer-events:none 不拦截画布指针事件 */}
                {!points.length && <div className="wave-canvas-hint">{t('result.wave.canvas_hint')}</div>}
            </div>
            <div className="lvl-row">
                <button type="button" className="lvl-btn" onClick={clear}>{t('result.wave.clear')}</button>
                <button
                    type="button"
                    className="lvl-btn primary"
                    disabled={points.length < MIN_POINTS}
                    onClick={() => fit(pointsRef.current)}
                >{t('result.wave.refit')}</button>
                {result && !result.ok && <span className="lvl-error">{result.msg}</span>}
            </div>
            {result?.ok && s && (
                <div className="wave-result">
                    {/* 拟合曲线预览：完整双正弦（拟合自手绘目标线），照抄前可先目检 */}
                    <div className="wave-preview" title={t('result.wave.preview_title')}>
                        <svg viewBox="0 0 100 100" preserveAspectRatio="none">
                            <path d={waveCurvePath(result)} />
                        </svg>
                    </div>
                    {/* 滑块可视化：与游戏四个滑块同构，滑钮位置即应照抄的位置 */}
                    {[
                        [t('result.wave.amp'), s.amp],
                        [t('result.wave.freq'), s.freq],
                        [t('result.wave.phase'), s.phase],
                        [t('result.wave.off'), s.off],
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

// ---------------- 第二关 密码破解 ----------------

export function CipherPage() {
    const { t } = useTranslation();
    const [session, setSession] = useState(loadSession);
    // 会话只经 rune-store 读写；写操作前重新加载，避免覆盖刚写入的记录。
    // 注意必须返回计算结果本身（setSession 无返回值，表达式体箭头函数会返回 undefined）
    const mutate = (fn) => {
        const next = fn(loadSession());
        setSession(next);
        return next;
    };
    const { history } = session;
    const [guess, setGuess] = useState([0, 1, 2, 3]);
    // 反馈标色默认全绿（v1.2.9）：不点击保留标色即视为绿，点按可修正
    const [colors, setColors] = useState([2, 2, 2, 2]);
    const [selected, setSelected] = useState(0);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    const worker = useRef(null);
    // 破解成功后自动开新一局的延迟定时器（600ms），卸载时清理避免泄漏
    const newGameTimer = useRef(null);
    const serial = useRef(0);
    const sessionRef = useRef(session); // 当前会话序列化快照，广播去重用
    // 符文名本地化（result.rune.*）：与 codes 序号同序，猜测槽/候选按钮/历史记录共用
    const symbolList = runeKeys.map((k) => t(`result.rune.${k}`));
    const label = (index) => codes[index].map((i) => symbolList[i]).join(' · ');

    useEffect(() => {
        const w = new Worker(new URL('../../utils/runes.worker.js', import.meta.url), { type: 'module' });
        worker.current = w;
        w.onmessage = ({ data }) => {
            if (data.id !== serial.current) return;
            setBusy(false);
            setError(data.error || '');
            // 给出新的最优猜测，重置反馈方块为默认绿等待用户录入
            if (!data.error && data.guess != null) setGuess([...codes[data.guess]]);
            setColors([2, 2, 2, 2]);
            setSelected(0);
        };
        w.onerror = () => {
            setBusy(false);
            setError(t('result.cipher.worker_error'));
        };
        // 挂载时若已有历史（上一窗口先记录过），重算推荐而不是停留在初始猜测
        if (session.history.length) {
            setBusy(true);
            w.postMessage({ id: ++serial.current, history: session.history });
        }
        return () => {
            window.clearTimeout(newGameTimer.current);
            w.terminate();
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // 写入 localStorage 并广播 SESSION_EVENT（其他订阅方据此刷新）
    useEffect(() => {
        sessionRef.current = session;
        saveSession(session);
    }, [session]);

    // 其他窗口新一局/写入记录时同步本页并重算推荐（storage 事件不发给同文档写入方）
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

    // 左键点反馈方块：循环 绿 → 红 → 黄，同时设为选中位
    const cycleColor = (i) => {
        setSelected(i);
        setColors((f) => f.map((v, j) => (j === i ? COLOR_CYCLE[(COLOR_CYCLE.indexOf(v) + 1) % COLOR_CYCLE.length] : v)));
    };

    // 中键=黄、右键=红：直接落到指定色（右键需拦下系统菜单）
    const markColor = (i, c) => (e) => {
        e.preventDefault();
        setSelected(i);
        setColors((f) => f.map((v, j) => (j === i ? c : v)));
    };

    // 点符文选项：改选中框符号并前进到下一框
    const pickRune = (n) => {
        setGuess((g) => g.map((v, j) => (j === selected ? n : v)));
        setSelected((s) => (s + 1) % 4);
    };

    // 修正：写入历史 → worker 重算新推荐（标色默认绿，无需再等齐全校验）
    const applyFix = () => {
        if (busy) return;
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
            newGameTimer.current = window.setTimeout(() => newGame(), 600);
        }
    };

    const newGame = () => {
        setGuess([0, 1, 2, 3]);
        setColors([2, 2, 2, 2]);
        setSelected(0);
        mutate(startNewGame);
        setBusy(true);
        worker.current?.postMessage({ id: ++serial.current, history: [] });
    };

    const undo = () => {
        if (!history.length) return;
        // 用 mutate 返回的最新会话发 worker（渲染闭包里的 history 可能落后于刚写入的记录）
        const next = mutate(undoLast);
        setBusy(true);
        worker.current.postMessage({ id: ++serial.current, history: next.history });
    };

    const turn = 1 + history.length; // 第几次猜测
    const ended = history.length >= 4;

    return (
        <div className="lvl-body">
            <div className="lvl-row between">
                <span className="lvl-status">{busy ? t('result.cipher.calculating') : error ||
                    (ended ? t('result.cipher.ended') : t('result.cipher.turn', { turn }))}</span>
                <span className="lvl-tools">
                    <button type="button" className="lvl-btn" onClick={newGame}>{t('result.cipher.new_game')}</button>
                    <button type="button" className="lvl-btn" disabled={!history.length} onClick={undo}>{t('result.cipher.undo')}</button>
                </span>
            </div>
            <p className="lvl-desc">
                <Trans
                    i18nKey="result.cipher.color_hint"
                    components={{
                        green: <b className="lvl-c2" />,
                        red: <b className="lvl-c0" />,
                        yellow: <b className="lvl-c1" />,
                    }}
                />
            </p>
            <div className="cipher-slots">
                {guess.map((value, i) => (
                    <button
                        type="button"
                        key={i}
                        className={`cipher-slot ${selected === i ? 'is-selected' : ''} is-${colors[i]}`}
                        onClick={() => cycleColor(i)}
                        onMouseDown={(e) => { if (e.button === 1) e.preventDefault(); }}
                        onAuxClick={markColor(i, 1)}
                        onContextMenu={markColor(i, 0)}
                        title={t('result.cipher.slot_color', { color: t(`result.color.${COLOR_KEYS[colors[i]]}`) })}
                    >
                        <span className="cipher-symbol">{symbolList[value]}</span>
                        <span className="cipher-color-name">{t(`result.color.${COLOR_KEYS[colors[i]]}`)}</span>
                    </button>
                ))}
            </div>
            <div className="cipher-options">
                {runeKeys.map((k, n) => <button type="button" key={k} onClick={() => pickRune(n)} disabled={ended}>{symbolList[n]}</button>)}
            </div>
            <button
                type="button"
                className="lvl-btn primary"
                disabled={busy || ended}
                onClick={applyFix}
            >{t('result.cipher.fix')}</button>
            {!!history.length && (
                <ol className="rune-history">
                    {history.map((h, i) => (
                        <li key={i}>
                            <span>{i + 1}. {label(h.guess)}</span>
                            <span>
                                {[0, 1, 2, 3].map((j) => (
                                    <b key={j} className={`lvl-c${Math.floor(h.result / 3 ** j) % 3}`}>
                                        {t(`result.color.${COLOR_KEYS[Math.floor(h.result / 3 ** j) % 3]}`)}
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
