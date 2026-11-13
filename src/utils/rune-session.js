import { codes } from './runes.js';

export function analyze(solver, history) {
    if (history.length > 4) {
        throw new Error('总提交次数不能超过四次。');
    }
    let candidates = codes.map((_, i) => i);
    let solved = false;
    for (const { guess, result } of history) {
        if (solved || !Number.isInteger(guess) || guess < 0 || guess >= codes.length ||
            !Number.isInteger(result) || result < 0 || result > 80) {
            throw new Error('无效的历史记录。');
        }
        candidates = candidates.filter(i => solver.matrix[guess][i] === result);
        solved = result === 80;
    }
    const turns = 4 - history.length;
    return {
        count: candidates.length, turns, solved: solved && candidates.length > 0,
        guess: solved ? null : solver.choose(candidates, turns),
    };
}
