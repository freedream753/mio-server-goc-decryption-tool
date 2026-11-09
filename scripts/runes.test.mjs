import test from 'node:test';
import assert from 'node:assert/strict';
import { codes, feedback, createSolver } from '../src/utils/runes.js';
import { analyze } from '../src/utils/rune-session.js';

test('duplicate symbols distinguish the two feedback rules', () => {
    assert.equal(feedback([5, 3, 3, 3], [3, 3, 5, 0], 'presence'), 43); // 黄绿黄黄
    assert.equal(feedback([5, 3, 3, 3], [3, 3, 5, 0], 'count'), 16); // 黄绿黄红
    assert.equal(feedback([0, 0, 0, 0], [0, 0, 0, 0], 'count'), 80);
});

for (const mode of ['presence', 'count']) {
    test(`${mode}: all 1296 secrets solved within four actual submissions`, () => {
        const solver = createSolver(mode);
        const histogram = [0, 0, 0, 0];
        function walk(candidates, turns) {
            const guess = solver.choose(candidates, turns);
            assert.notEqual(guess, null, `${candidates.length} candidates, ${turns} turns`);
            const buckets = new Map();
            for (const secret of candidates) {
                const result = solver.matrix[guess][secret];
                if (result === 80) { histogram[4 - turns]++; continue; }
                if (!buckets.has(result)) buckets.set(result, []);
                buckets.get(result).push(secret);
            }
            for (const group of buckets.values()) {
                assert.ok(turns > 1);
                walk(group, turns - 1);
            }
        }
        walk(codes.map((_, i) => i), 4);
        assert.equal(histogram.reduce((a, b) => a + b), 1296);
        assert.deepEqual(histogram, mode === 'presence' ? [1, 26, 818, 451] : [1, 20, 918, 357]);
        assert.equal(analyze(solver, [], 3).guess, null);
        assert.equal(analyze(solver, [{ guess: 51, result: 80 }]).solved, true);
        assert.equal(analyze(solver, [{ guess: 0, result: 0 }, { guess: 0, result: 80 }]).count, 0);
        assert.equal(analyze(solver, [{ guess: 0, result: 0 }], 3).turns, 0);
        assert.throws(() => analyze(solver, [{ guess: 0, result: 0 }], 4));
        assert.throws(() => analyze(solver, [{ guess: 1296, result: 0 }]));
        console.log(mode, histogram);
    });
}
