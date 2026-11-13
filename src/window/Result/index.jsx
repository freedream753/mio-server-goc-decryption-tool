// 三关卡集成窗口（v1.2.7：页签重排为 校准/西蒙/解密；标题旁框选按钮改为全局
// 裁切模式开关——框选屏幕区域后三条截图管线只采集框内画面，按钮切换为「取消
// 裁切模式」；v1.2.2 删除「识别」页——与「解密」页是同一套符文解谜，Alt+V 自动
// 识别结果直接写入解密页会话并重算推荐，识别不完整时不写历史，由用户在解密页
// 手动录入；v1.2.0 起无自动倒计时，由用户手动关闭）
// 数据流：Rust 识别（裁切区域内）→ emit `rune_scan` + 静态 RUNE_SCAN，挂载后主动
//        `get_rune_scan()` 拉一次防竞态；
//        payload = { ok, slots:[{rune,color}×4], source, msg }，rune 0=空 1=火 2=水 3=风 4=土 5=雷，color 0=红 1=黄 2=绿。
// 8 项全部非 null 才自动追加一条历史（与末条完全相同则跳过，同一屏重复按热键不重复记）。
// 解密页求解走自己的 Web Worker（runes.worker.js + runes.js 的 createSolver），主线程不建 1296×1296 矩阵。
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { version } from '../../../package.json';
import { appendRecord, loadSession, recordFromSlots, saveSession } from '../../utils/rune-store';
import { CipherPage, SimonPage, WavePage } from './pages';
import './style.css';

// 页签定义（v1.2.7 重排）：校准（第一关波形，Alt+X）+ 西蒙（第二关，Alt+C）+ 解密（Alt+V）；
// 页签名经 i18n 取自 result.tab.<id>（其他 locale 缺失时按 fallbackLng 回落）；
// 窗口高度由 ResizeObserver 跟随内容实际高度动态调整（见下），命中区域 = 可视区域
const TABS = ['wave', 'simon', 'cipher'];

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
    const { t } = useTranslation();
    const [tab, setTab] = useState('wave');
    const [cropActive, setCropActive] = useState(false);
    // 最近一次符文识别的失败信息（ok=false 时展示在页签下方，成功后清除）
    const [runeError, setRuneError] = useState('');
    const rootRef = useRef(null);
    const lastHeight = useRef(0);
    // 西蒙录制运行状态镜像：切到非西蒙页签时自动暂停录制用
    const simonRunningRef = useRef(false);

    // 切页统一入口：西蒙录制中切到校准/解密页签 → 自动暂停（录制仅限西蒙页签可见）
    const changeTab = (next) => {
        if (next !== 'simon' && simonRunningRef.current) {
            invoke('simon_stop').catch(() => {});
        }
        setTab(next);
    };

    // rune_scan payload → 完整记录自动写入会话（解密页经 SESSION_EVENT 刷新并重算推荐）；
    // 无论成败都切到解密页签（Alt+V 语义就是「识别解密」，失败信息展示在解密页签下方）；
    // 识别不完整（ok 但槽位缺失）时不写历史，等用户在解密页手动录入
    const applyPayload = (payload) => {
        if (!payload) return;
        if (!payload.ok) {
            setRuneError(payload.msg || t('result.rune_scan_failed'));
            changeTab('cipher');
            return;
        }
        setRuneError('');
        changeTab('cipher');
        const next = normalizeSlots(payload.slots);
        const record = recordFromSlots(next.map((s) => s.rune), next.map((s) => s.color));
        if (!record) return;
        const appended = appendRecord(loadSession(), record);
        if (appended.added) saveSession(appended.session);
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

    // 后台管线触发时自动切到对应关卡页（数据由各页面自己的监听接收）：
    // 波形求解 → wave_scan；西蒙热键开始录制 → simon_state(running=true)。
    // 西蒙首次热键触发时窗口可能刚创建、emit 早于监听注册，
    // 挂载后拉 get_simon_state 兜底（已在录制则切到西蒙页签）
    useEffect(() => {
        invoke('get_simon_state')
            .then((running) => {
                simonRunningRef.current = !!running;
                if (running) changeTab('simon');
            })
            .catch(() => {});
        const unWave = listen('wave_scan', () => changeTab('wave'));
        const unSimon = listen('simon_state', (e) => {
            simonRunningRef.current = !!e.payload?.running;
            if (e.payload?.running) changeTab('simon');
        });
        return () => {
            unWave.then((f) => f());
            unSimon.then((f) => f());
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // 全局裁切模式状态：挂载拉取一次（防竞态）+ 监听变更，按钮随之切换
    useEffect(() => {
        invoke('get_crop_state').then(setCropActive).catch(() => {});
        const un = listen('crop_state', (e) => setCropActive(!!e.payload));
        return () => {
            un.then((f) => f());
        };
    }, []);

    // 命中区域 = 可视区域（v1.2.7）：窗口高度跟随内容实际高度，
    // 消除「窗口实际占用区域比可视化区域大，点击下方被透明区域拦截」的问题。
    // 仅在高度变化超过 1px 时上报，避免 resize 反馈环。
    // 上报高度 = 内容完整高度 =（容器除活动页外的固定部分）+（活动页 scrollHeight）。
    // 不能直接用容器 offsetHeight：容器 max-height:100vh 在视口内钳制后 offsetHeight
    // 恒等于视口高，窗口永远长不大（解密页内容被裁回归）。活动页 scrollHeight
    // 在页内滚动激活时仍返回完整内容高，因此：
    //   内容 ≤ 视口：body 不滚动，scrollHeight=offsetHeight → 上报=容器实高（原行为）
    //   内容 > 视口：上报完整内容高 → 窗口长高，直到被 Rust 按显示器钳制 →
    //               页内滚动接管，上报值不随窗口收缩（无逐级下探反馈环）
    useEffect(() => {
        const el = rootRef.current;
        if (!el || typeof ResizeObserver === 'undefined') return undefined;
        const measure = () => {
            const body = el.querySelector('.lvl-wrap:not(.is-hidden) .lvl-body');
            const h = body
                ? el.offsetHeight - body.offsetHeight + body.scrollHeight
                : el.offsetHeight;
            if (Math.abs(h - lastHeight.current) > 1) {
                lastHeight.current = h;
                invoke('set_result_size', { width: 420, height: h }).catch(() => {});
            }
        };
        const ro = new ResizeObserver(measure);
        ro.observe(el);
        // 三个页面常驻挂载：显示/隐藏与内容变化都会触发对应回调（隐藏页无盒子不触发）
        el.querySelectorAll('.lvl-body').forEach((b) => ro.observe(b));
        return () => ro.disconnect();
    }, []);

    const closeResult = () => {
        invoke('hide_result_window').catch(() => {});
    };

    // 全局裁切模式开关：框选屏幕区域（之后校准/西蒙/解密只识别框内画面）⇄ 取消裁切
    const toggleCrop = () => {
        invoke(cropActive ? 'crop_clear' : 'crop_select').catch(() => {});
    };

    return (
        <div className="result-container" ref={rootRef}>
            <div className="result-header" data-tauri-drag-region>
                {/* Tauri v1 拖拽区只识别 mousedown 的精确 target，header 子元素必须逐个带上属性 */}
                <span className="result-grip" data-tauri-drag-region title={t('result.drag')}>⠿</span>
                {/* 裁切模式开关（v1.2.7）：激活时图标实心 + 橙色，悬停看说明 */}
                <span
                    className={`result-ocr ${cropActive ? 'is-active' : ''}`}
                    title={cropActive ? t('result.crop_off') : t('result.crop_on')}
                    onClick={toggleCrop}
                >
                    {cropActive ? (
                        <svg width="11" height="11" viewBox="0 0 12 12" fill="currentColor">
                            <rect x="2" y="2" width="8" height="8" rx="1.5" />
                        </svg>
                    ) : (
                        <svg width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                            <path d="M1 4 V1 H4" />
                            <path d="M8 1 H11 V4" />
                            <path d="M11 8 V11 H8" />
                            <path d="M4 11 H1 V8" />
                        </svg>
                    )}
                </span>
                <span className="result-title" data-tauri-drag-region>{t('result.title', { version })}</span>
                {/* 关闭按钮：不带拖拽区属性 */}
                <span className="result-close" title={t('result.close')} onClick={closeResult}>✕</span>
            </div>

            {/* 页签：校准/西蒙/解密，全部页面常驻挂载，隐藏仅 display:none */}
            <nav className="result-tabs">
                {TABS.map((id) => (
                    <button
                        type="button"
                        key={id}
                        className={`result-tab ${tab === id ? 'active' : ''}`}
                        onClick={() => changeTab(id)}
                    >{t(`result.tab.${id}`)}</button>
                ))}
            </nav>

            {/* 符文识别失败提示：挂载拉取/热键触发的 ok=false payload 都会落到这里 */}
            {runeError && <p className="lvl-error rune-scan-error">{runeError}</p>}

            <div className={`lvl-wrap ${tab === 'wave' ? '' : 'is-hidden'}`}><WavePage /></div>
            <div className={`lvl-wrap ${tab === 'simon' ? '' : 'is-hidden'}`}><SimonPage /></div>
            <div className={`lvl-wrap ${tab === 'cipher' ? '' : 'is-hidden'}`}><CipherPage /></div>
        </div>
    );
}
