import React, { useEffect, useRef, useState } from 'react';
import { appWindow } from '@tauri-apps/api/window';
import { codes, symbols } from '../../utils/runes';
import {
    appendRecord, loadSession, saveSession, setMode, setSpent, startNewGame, undoLast,
} from '../../utils/rune-store';
import './style.css';

const colors = ['红', '黄', '绿'];
const label = index => codes[index].map(i => symbols[i]).join(' · ');

export default function Runes() {
    const [session, setSession] = useState(loadSession);
    // 会话只经 rune-store 读写；写操作前重新加载，避免覆盖结果窗口刚写入的自动识别记录
    const mutate = fn => setSession(fn(loadSession()));
    const { mode, history, spent } = session;
    const [guess, setGuess] = useState([0, 1, 2, 3]);
    const [feedback, setFeedback] = useState([null, null, null, null]);
    const [result, setResult] = useState(null);
    const [busy, setBusy] = useState(true);
    const [error, setError] = useState('');
    const worker = useRef(null);
    const serial = useRef(0);

    useEffect(() => {
        const w = new Worker(new URL('../../utils/runes.worker.js', import.meta.url), { type: 'module' });
        worker.current = w;
        w.onmessage = ({ data }) => {
            if (data.id !== serial.current) return;
            setBusy(false);
            setError(data.error || '');
            setResult(data.error ? null : data);
            if (data.guess != null) setGuess([...codes[data.guess]]);
        };
        w.onerror = () => { setBusy(false); setError('策略计算失败，请关闭窗口后重新打开。'); };
        return () => w.terminate();
    }, []);

    useEffect(() => {
        setBusy(true);
        setError('');
        setResult(null);
        setFeedback([null, null, null, null]);
        saveSession(session);
        worker.current.postMessage({ id: ++serial.current, ...session });
    }, [session]);

    function submit() {
        if (busy || feedback.includes(null) || !result?.count || result.solved || result.turns < 1) return;
        const encoded = guess.reduce((n, v) => n * 6 + v, 0);
        const encodedFeedback = feedback.reduce((n, v, i) => n + v * 3 ** i, 0);
        mutate(s => appendRecord(s, { guess: encoded, result: encodedFeedback }).session);
    }
    const ended = result?.solved || result?.turns === 0;
    const status = busy ? '正在计算可保证的策略…' : error ||
        (result?.count === 0 ? '反馈与当前规则或历史矛盾。请撤回检查，或切换判色规则。' :
        result?.solved ? '解密成功！开始下一局前请点击「新一局」。' :
        result?.turns === 0 ? '四次机会已用完，请开始新一局。' :
        result?.guess == null ? '剩余次数内无法保证解出。可撤回检查，或录入下一次实际猜测继续筛选。' :
        `推荐提交：${label(result.guess)}。在游戏中提交后，录入反馈。`);

    return <main className='runes'>
        <header className='rune-header' data-tauri-drag-region>
            <strong data-tauri-drag-region>GOC · 四位符文解密</strong>
            <button onClick={() => appWindow.hide()} title='隐藏窗口，保留本局'>隐藏</button>
        </header>
        <div className='rune-body'>
            <p>六种符文可重复，共四次机会。此助手手动录入游戏反馈，不会替你点击游戏。</p>
            <div className='rune-toolbar'>
                <label>判色规则 <select value={mode} onChange={e => mutate(s => setMode(s, e.target.value))}>
                    <option value='count'>按数量匹配</option><option value='presence'>存在即黄</option>
                </select></label>
                <button onClick={() => { setGuess([0, 1, 2, 3]); mutate(startNewGame); }}>新一局</button>
                <button disabled={!history.length} onClick={() => mutate(undoLast)}>撤回反馈</button>
            </div>
            <p className='rune-rule'>{mode === 'presence' ? '绿：位置正确；黄：答案中有该符文但位置不同（不扣数量）；红：答案中没有该符文。' : '先匹配绿色，再从左到右按剩余数量匹配黄色；红色也可能表示该符文数量已用完。'}</p>
            <label className='rune-spent'>未录入的已用次数 <select value={spent} disabled={history.length > 0} onChange={e => mutate(s => setSpent(s, e.target.value))}>
                {[0, 1, 2, 3].map(n => <option key={n} value={n}>{n}</option>)}
            </select> <small>中途接入时设置；已录入的记录会另计次数。</small></label>
            <section className='rune-status' role='status' aria-live='polite'>
                <div>剩余机会 <b>{4 - spent - history.length}</b> / 4　候选 <b>{result?.count ?? '…'}</b> / 1296</div>
                <p>{status}</p>
            </section>
            <h2>当前实际猜测与反馈（从左到右）</h2>
            <div className='rune-slots'>
                {guess.map((value, i) => <div className='rune-slot' key={i}>
                    <label>第 {i + 1} 位<select aria-label={`第${i + 1}位符文`} value={value} disabled={busy || ended} onChange={e => {
                        setGuess(g => g.map((v, j) => j === i ? Number(e.target.value) : v));
                        setFeedback(f => f.map((v, j) => j === i ? null : v));
                    }}>{[...symbols].map((s, n) => <option value={n} key={s}>{s}</option>)}</select></label>
                    <div className='rune-colors'>{colors.map((c, n) => <button key={c} className={`rune-color c${n} ${feedback[i] === n ? 'selected' : ''}`} aria-label={`第${i + 1}位${c}色`} aria-pressed={feedback[i] === n} disabled={busy || ended} onClick={() => setFeedback(f => f.map((v, j) => j === i ? n : v))}>{c}</button>)}</div>
                </div>)}
            </div>
            <button className='rune-submit' disabled={busy || ended || !result?.count || feedback.includes(null)} onClick={submit}>记录游戏反馈 · 计算下一步</button>
            <p className='rune-note'>四次保证仅适用于新局按推荐操作、反馈准确且判色规则一致的情况。已有猜测可直接修改四个符文后录入。</p>
            <h2>本局记录</h2>
            {!history.length && <p>暂无记录。新局首猜：空 · 火 · 水 · 风。</p>}
            <ol className='rune-history'>{history.map((h, i) => <li key={i}>
                <span>第 {spent + i + 1} 次　{label(h.guess)}</span>
                <span>{[0, 1, 2, 3].map(j => <b key={j} className={`c${Math.floor(h.result / 3 ** j) % 3}`}>{colors[Math.floor(h.result / 3 ** j) % 3]}</b>)}</span>
            </li>)}</ol>
        </div>
    </main>;
}
