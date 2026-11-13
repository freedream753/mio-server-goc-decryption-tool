import { createSolver } from './runes';
import { analyze } from './rune-session';

let solver = null;
self.onmessage = ({ data: { id, history } }) => {
    try {
        solver ??= createSolver();
        self.postMessage({ id, ...analyze(solver, history) });
    } catch (error) {
        self.postMessage({ id, error: error.message });
    }
};
