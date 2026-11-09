// 本局符文会话的唯一存取入口（localStorage 键 goc-rune-session-v1）。
// Runes 手动助手与 Result 自动识别窗口都必须经这里读写，避免两边各自 getItem/setItem 互相覆盖。
// 会话结构（与 Rust 侧契约一致）：
//   mode: 'presence' | 'count'
//   history: [{ guess, result }]，guess = Σ rune_i * 6^(3-i)（0..1295），result = Σ color_i * 3^i（0..80）
//   spent: 0..3（中途接入时已用掉的次数）

const SESSION_KEY = 'goc-rune-session-v1';
const MODES = ['presence', 'count'];
const MAX_TURNS = 4;
const SLOT_COUNT = 4;
const RUNE_COUNT = 6;
const COLOR_COUNT = 3;
const SECRET_COUNT = RUNE_COUNT ** SLOT_COUNT; // 1296
const MAX_RESULT = COLOR_COUNT ** SLOT_COUNT - 1; // 80

// 默认判色规则为「按数量匹配」：实拍样张 雷风风风 / 黄绿黄红（result=16）在 presence 下 count=0 自相矛盾，
// 只有 count 下有解（9 个候选），说明游戏按剩余数量消耗式匹配黄色。
function defaultSession() {
    return { mode: 'count', history: [], spent: 0 };
}

function isValidRecord(record) {
    return (
        !!record &&
        Number.isInteger(record.guess) &&
        record.guess >= 0 &&
        record.guess < SECRET_COUNT &&
        Number.isInteger(record.result) &&
        record.result >= 0 &&
        record.result <= MAX_RESULT
    );
}

function isValidSession(value) {
    return (
        !!value &&
        MODES.includes(value.mode) &&
        Array.isArray(value.history) &&
        value.history.length <= MAX_TURNS &&
        Number.isInteger(value.spent) &&
        value.spent >= 0 &&
        value.spent <= MAX_TURNS - 1 &&
        value.history.length + value.spent <= MAX_TURNS &&
        value.history.every(isValidRecord)
    );
}

// 读取：存档缺失/损坏/越界一律回落为新局默认值
function loadSession() {
    try {
        const value = JSON.parse(localStorage.getItem(SESSION_KEY));
        if (isValidSession(value)) {
            return {
                mode: value.mode,
                history: value.history.map(({ guess, result }) => ({ guess, result })),
                spent: value.spent,
            };
        }
    } catch {
        /* 存档不可用：按新局处理。 */
    }
    return defaultSession();
}

// 写入：存储配额/隐私模式失败不阻断解题
function saveSession(session) {
    try {
        localStorage.setItem(SESSION_KEY, JSON.stringify(session));
    } catch {
        /* 保存失败不阻断解密。 */
    }
    return session;
}

function remainingTurns(session) {
    return MAX_TURNS - session.spent - session.history.length;
}

// 历史末条即本次记录：同一屏重复按热键不应重复记一条
function isRecorded(session, guess, result) {
    const last = session.history[session.history.length - 1];
    return !!last && last.guess === guess && last.result === result;
}

// 追加一次记录；与末条相同 / 次数已满 / 记录非法时原样返回
function appendRecord(session, record) {
    if (!isValidRecord(record) || remainingTurns(session) <= 0 || isRecorded(session, record.guess, record.result)) {
        return { session, added: false };
    }
    return {
        session: { ...session, history: [...session.history, { guess: record.guess, result: record.result }] },
        added: true,
    };
}

// 撤回末条（自动识别错判的人工兜底）
function undoLast(session) {
    return session.history.length ? { ...session, history: session.history.slice(0, -1) } : session;
}

// 新一局：保留判色规则，清空历史与已用次数
function startNewGame(session) {
    return { mode: session.mode, history: [], spent: 0 };
}

function setMode(session, mode) {
    return MODES.includes(mode) ? { ...session, mode } : session;
}

function setSpent(session, spent) {
    const value = Number(spent);
    if (!Number.isInteger(value) || value < 0 || value > MAX_TURNS - 1 || session.history.length + value > MAX_TURNS) {
        return session;
    }
    return { ...session, spent: value };
}

// 四位符文 -> guess：与 runes.js 的 codes 同序（第 1 位权重最大）
function encodeRunes(values) {
    return values.reduce((n, value) => n * RUNE_COUNT + value, 0);
}

// 四位反馈色 -> result：第 1 位权重最小（1/3/9/27）
function encodeColors(values) {
    return values.reduce((n, value, i) => n + value * COLOR_COUNT ** i, 0);
}

// 8 项齐全才返回可写入的记录，任一为 null 返回 null（等用户补齐后手点「记录本次」）
function recordFromSlots(runes, colors) {
    if (!Array.isArray(runes) || !Array.isArray(colors) || runes.length !== SLOT_COUNT || colors.length !== SLOT_COUNT) {
        return null;
    }
    const complete =
        runes.every((v) => Number.isInteger(v) && v >= 0 && v < RUNE_COUNT) &&
        colors.every((v) => Number.isInteger(v) && v >= 0 && v < COLOR_COUNT);
    return complete ? { guess: encodeRunes(runes), result: encodeColors(colors) } : null;
}

export {
    SESSION_KEY,
    MODES,
    MAX_TURNS,
    SLOT_COUNT,
    RUNE_COUNT,
    COLOR_COUNT,
    SECRET_COUNT,
    defaultSession,
    isValidRecord,
    isValidSession,
    loadSession,
    saveSession,
    remainingTurns,
    isRecorded,
    appendRecord,
    undoLast,
    startNewGame,
    setMode,
    setSpent,
    encodeRunes,
    encodeColors,
    recordFromSlots,
};
