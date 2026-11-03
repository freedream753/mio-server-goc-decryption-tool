const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const root = path.join(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'src/utils/store.js'), 'utf8')
    .replace(/^import .*;\r?\n/gm, '').replace(/export /g, '');

function setup({ missing = false, listenFails = false, pathFails = false } = {}) {
    const state = { subscriptions: 0, loads: 0, warnings: [], renders: 0, missing };
    const ctx = vm.createContext({
        Store: class { async load() { state.loads++; if (state.missing) throw Error('file missing'); } },
        appConfigDir: async () => { if (pathFails) throw Error('path unavailable'); return 'config'; },
        join: async (...parts) => parts.join('/'),
        listen: async (name, callback) => {
            state.subscriptions++; state.event = name; state.callback = callback;
            if (listenFails) throw Error('listener unavailable');
            return () => {};
        },
        console: { warn: (...args) => state.warnings.push(args) },
        initEnv: async () => {}, document: { getElementById: () => ({}) },
        ReactDOM: { createRoot: () => ({ render: () => { state.renders++; } }) },
    });
    vm.runInContext(source, ctx);
    return { state, ctx, init: () => vm.runInContext('initStore()', ctx) };
}

test('initializing repeatedly shares one subscription and one initial load', async () => {
    const { state, init } = setup();
    const first = init();
    for (let i = 0; i < 100; i++) assert.equal(init(), first);
    await first;
    assert.equal(state.subscriptions, 1);
    assert.equal(state.loads, 1);
    assert.equal(state.event, 'config_file_changed');
});

test('missing configuration is nonfatal and reloads after recreation', async () => {
    const { state, init } = setup({ missing: true });
    await init();
    assert.equal(state.warnings.length, 1);
    await state.callback(); // Still missing: no rejected callback or poisoned startup.
    state.missing = false;
    await state.callback();
    assert.equal(state.loads, 3);
    assert.equal(state.warnings.length, 2);
});

test('event subscription failure still loads cached configuration', async () => {
    const { state, init } = setup({ listenFails: true });
    await init();
    assert.equal(state.loads, 1);
    assert.equal(state.warnings.length, 1);
});

test('the actual startup chain renders after optional configuration initialization fails', async () => {
    const { state, ctx } = setup({ pathFails: true });
    const main = fs.readFileSync(path.join(root, 'src/main.jsx'), 'utf8');
    const start = main.indexOf('initStore().catch(');
    const end = main.indexOf('    root.render(', start);
    assert(start >= 0 && end > start);
    vm.runInContext('globalThis.startup = ' + main.slice(start, end) + 'root.render("test");\n});', ctx);
    await ctx.startup;
    assert.equal(state.renders, 1);
    assert.equal(state.warnings.length, 1);
});
