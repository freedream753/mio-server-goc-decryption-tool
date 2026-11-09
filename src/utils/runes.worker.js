import { createSolver } from './runes';
import { analyze } from './rune-session';

const solvers = {};
self.onmessage = ({ data: { id, mode, history, spent } }) => {
    try {
        if (!['presence', 'count'].includes(mode)) throw new Error('未知判色规则');
        const solver = solvers[mode] ??= createSolver(mode);
        self.postMessage({ id, ...analyze(solver, history, spent) });
    } catch (error) {
        self.postMessage({ id, error: error.message });
    }
};
