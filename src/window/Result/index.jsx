// 两关卡集成窗口（noocr 支线：无截图/OCR，西蒙页与全局裁切模式已移除）：
// 页签 = 校准（第一关波形手绘）+ 解密（第三关）。
// 数据流：波形页手绘 → wave_fit_points（页面内自治）；解密页会话经 rune-store 读写。
// 呼出链路：热键/托盘/HTTP 呼出时后端写入待切页签 → 本窗口挂载读 get_open_tab
//（读即清防竞态）并监听 open_tab 事件切页。
import { invoke } from '@tauri-apps/api';
import { listen } from '@tauri-apps/api/event';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { version } from '../../../package.json';
import { CipherPage, WavePage } from './pages';
import './style.css';

// 页签定义：校准（第一关波形）+ 解密（第三关）；
// 页签名经 i18n 取自 result.tab.<id>（其他 locale 缺失时按 fallbackLng 回落）；
// 窗口高度由 ResizeObserver 跟随内容实际高度动态调整（见下），命中区域 = 可视区域
const TABS = ['wave', 'cipher'];

export default function Result() {
    const { t } = useTranslation();
    const [tab, setTab] = useState('wave');
    const rootRef = useRef(null);
    const lastHeight = useRef(0);

    // 切页统一入口
    const changeTab = (next) => {
        setTab(next);
    };

    // 热键/托盘/HTTP 呼出时后端写入待切页签：挂载先读 get_open_tab（读即清防竞态），
    // 再挂 open_tab 事件监听
    useEffect(() => {
        invoke('get_open_tab').then((next) => { if (next) changeTab(next); }).catch(() => {});
        const un = listen('open_tab', (e) => changeTab(e.payload));
        return () => { un.then((f) => f()); };
        // eslint-disable-next-line react-hooks/exhaustive-deps
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
        // 两个页面常驻挂载：显示/隐藏与内容变化都会触发对应回调（隐藏页无盒子不触发）
        el.querySelectorAll('.lvl-body').forEach((b) => ro.observe(b));
        return () => ro.disconnect();
    }, []);

    const closeResult = () => {
        invoke('hide_result_window').catch(() => {});
    };

    return (
        <div className="result-container" ref={rootRef}>
            <div className="result-header" data-tauri-drag-region>
                {/* Tauri v1 拖拽区只识别 mousedown 的精确 target，header 子元素必须逐个带上属性 */}
                <span className="result-grip" data-tauri-drag-region title={t('result.drag')}>⠿</span>
                <span className="result-title" data-tauri-drag-region>{t('result.title', { version })}</span>
                {/* 关闭按钮：不带拖拽区属性 */}
                <span className="result-close" title={t('result.close')} onClick={closeResult}>✕</span>
            </div>

            {/* 页签：校准/解密，全部页面常驻挂载，隐藏仅 display:none */}
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

            <div className={`lvl-wrap ${tab === 'wave' ? '' : 'is-hidden'}`}><WavePage /></div>
            <div className={`lvl-wrap ${tab === 'cipher' ? '' : 'is-hidden'}`}><CipherPage /></div>
        </div>
    );
}
