// 三关卡页面（v1.2.4：转轮页删除；v1.2.1 集成进热键结果窗口）：
// 密码破解 / 波形校准 / 西蒙符印。
// 样式沿用结果窗口紧凑暗色风格（lvl- 前缀，见 style.css）。
// 三个页面在 Result 里常驻挂载（隐藏仅 display:none），保证西蒙录制的
// 事件监听不随页签切换丢失，录制状态得以保留。
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
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
    const { t } = useTranslation();
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
    // 手动框选降级已封存（v1.4.0 全屏实现弃用）：识别统一走全局裁切模式
    const s = result?.sliders;
    return (
        <div className="lvl-body">
            <p className="lvl-desc">{t('result.wave.desc')}</p>
            <div className="lvl-row">
                <button type="button" className="lvl-btn primary" onClick={scan}>{t('result.wave.scan')}</button>
                {result && !result.ok && <span className="lvl-error">{result.msg}</span>}
            </div>
            {result?.ok && s && (
                <div className="wave-result">
                    {/* 反推曲线预览：完整双正弦（拟合自游戏目标线），照抄前可先目检 */}
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

// ---------------- 第二关 西蒙符印 ----------------

// 西蒙 3×2 格子布局（上排 火水风、下排 土雷光，与 Rust 侧步骤 index 约定同序：
// 显示经 result.rune.* 取本地化名（光在西蒙语境存在，单独补 light）
const SIMON_KEYS = ['fire', 'water', 'wind', 'earth', 'thunder', 'light'];

export function SimonPage() {
    const { t } = useTranslation();
    const [steps, setSteps] = useState([]);
    const [running, setRunning] = useState(false);
    const [error, setError] = useState('');
    const [info, setInfo] = useState('');
    // 步骤列表同步代际号：只应用最后一次发起的 refetch 结果，防止并发
    // invoke 的响应乱序返回时旧快照覆盖新列表
    const syncSeq = useRef(0);
    const syncSteps = () => {
        const id = ++syncSeq.current;
        invoke('get_simon_steps')
            .then((s) => {
                if (id === syncSeq.current) setSteps(s);
            })
            .catch(() => {});
    };

    useEffect(() => {
        syncSteps();
        // simon_step 不做本地乐观追加：轮次截断会丢弃超量步骤，其已排队的
        // step 事件若按 seq 去重追加会让格子短暂多显示一个后端不存在的序号；
        // 统一走全量 refetch（本地 IPC 毫秒级），前端列表恒等于后端权威列表
        const unStep = listen('simon_step', () => syncSteps());
        const unState = listen('simon_state', (e) => {
            setRunning(e.payload.running);
            setError(e.payload.error || '');
            setInfo(e.payload.info || '');
            // 每次状态事件都以 Rust 侧列表为准同步：自动模式切轮/出错时后端已
            // 清空步骤并从 1 重新计数，本地旧列表不清会导致新一轮 seq 撞号；
            // 停止时兼做补齐（停止事件可能早于最后一帧的 step）
            syncSteps();
        });
        return () => {
            unStep.then((f) => f());
            unState.then((f) => f());
        };
    }, []);

    const stop = () => invoke('simon_stop').catch(() => {});
    // 自动模式（默认）：后端按游戏状态区文字（开始/正在播放/轮次X/3/错误）自动
    // 待机、采集与截断，切到本页签时由 index.jsx 自动启动；此处按钮为手动重启
    const startAuto = () => {
        setSteps([]);
        setError('');
        setInfo('');
        invoke('simon_start_auto').catch(() => {});
    };
    // 自由录制：不限步数、不识别状态文字，手动启停（原三关式 4/5/6 按钮已移除）
    const startFree = () => {
        setSteps([]);
        setError('');
        setInfo('');
        invoke('simon_start', { target: 0 }).catch(() => {});
    };

    return (
        <div className="lvl-body">
            <p className="lvl-desc">{t('result.simon.desc')}</p>
            <div className="lvl-row">
                {running ? (
                    <>
                        <button type="button" className="lvl-btn primary is-danger" onClick={stop}>{t('result.simon.stop')}</button>
                        <span className="lvl-hint">{t('result.simon.recording', { current: steps.length, total: '' })}</span>
                    </>
                ) : (
                    <>
                        <button type="button" className="lvl-btn primary" onClick={startAuto}>{t('result.simon.auto')}</button>
                        <button type="button" className="lvl-btn" onClick={startFree}>{t('result.simon.free')}</button>
                    </>
                )}
                {error && <span className="lvl-error">{error}</span>}
                {!error && info && <span className="lvl-hint">{info}</span>}
            </div>
            {/* 3×2 步骤格（与游戏同布局）：格子中心大字标该符文亮起的顺序序号，
                录制中实时点亮；不用旧的「1. 名 2. 名」排序条 */}
            {(!!steps.length || running) && (
                <>
                    <div className="simon-grid">
                        {SIMON_KEYS.map((k, i) => {
                            const seqs = steps.filter((s) => s.index === i).map((s) => s.seq);
                            return (
                                <div className="simon-tile" key={k}>
                                    <span className="simon-tile-name">{t(`result.rune.${k}`)}</span>
                                    <span className={`simon-tile-seq ${seqs.length ? '' : 'is-empty'}`}>
                                        {seqs.length ? seqs.join(' · ') : '–'}
                                    </span>
                                </div>
                            );
                        })}
                    </div>
                    <p className="lvl-hint">{t('result.simon.grid_hint')}</p>
                </>
            )}
            {!steps.length && !running && <p className="lvl-hint">{t('result.simon.empty')}</p>}
        </div>
    );
}

// ---------------- 第三关 密码破解 ----------------

export function CipherPage() {
    const { t } = useTranslation();
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
        // 挂载时若已有历史（自动识别或上一窗口先记录过），重算推荐而不是停留在初始猜测
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
        // 用 mutate 返回的最新会话发 worker（渲染闭包里的 history 可能落后于刚写入的自动识别记录）
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
                    {/* 识别：与热键 Alt+V 同入口，全屏识别结果自动写入本页并重算推荐 */}
                    <button type="button" className="lvl-btn" onClick={() => invoke('rune_recognize').catch(() => {})}>{t('result.cipher.recognize')}</button>
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
