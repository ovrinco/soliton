import fs from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';

function extract(source, name) {
  const start = source.lastIndexOf(`function ${name}(`);
  assert(start >= 0, `missing ${name}`);
  let depth = 0, quote = '', escape = false, line = false, block = false;
  for (let i = source.indexOf('{', start); i < source.length; i++) {
    const c = source[i], n = source[i + 1];
    if (line) { if (c === '\n') line = false; continue; }
    if (block) { if (c === '*' && n === '/') { block = false; i++; } continue; }
    if (quote) { if (escape) escape = false; else if (c === '\\') escape = true; else if (c === quote) quote = ''; continue; }
    if (c === '/' && n === '/') { line = true; i++; continue; }
    if (c === '/' && n === '*') { block = true; i++; continue; }
    if (c === '"' || c === "'" || c === '`') { quote = c; continue; }
    if (c === '{') depth++;
    if (c === '}' && --depth === 0) return source.slice(start, i + 1).replace(/([A-Za-z_$][\w$]*)\s*:\s*(?:string|int|bool|real|var)\b/g, '$1');
  }
  throw Error(`unterminated ${name}`);
}

function fixture(source) {
  const requests = [];
  const base = { id: 'task-1', hash: 'hash-1', revision: 8, title: 'Series title', status: 'todo', recurrence: 'weekly' };
  const backend = {
    records: [base], events: [], drafts: {}, activeKey: '', selectedId: '', saving: false,
    detailsOpen: false, detailRoute: '', routeKey: 'Tasks/List', compactPane: '',
    restored: true, prefsDirty: false, messageCalls: [],
    get(id) { return this.records.find(r => r.id === id) || null; },
    remember() {}, message(text, error) { this.messageCalls.push({ text, error }); }, send(command, args, cb) { requests.push({ command, args, cb }); }
  };
  Object.defineProperty(backend, 'activeDraft', { get() { return this.drafts[this.activeKey] || null; } });
  const context = vm.createContext({ b: backend, JSON, Object, Array, String });
  for (const name of ['select', 'saveDraft', 'reloadDraft']) {
    const fn = vm.runInContext(`(function(){ with(b) { return (${extract(source, name)}); } })()`, context);
    backend[name] = fn;
  }
  const occA = { id: 'task-1', occurrence: 'occ-A', occurrence_date: '2026-09-28', occurrence_sequence: 12, date: '2026-09-28', schedule: null, title: 'Occurrence title', status: 'completed' };
  const occB = { id: 'task-1', occurrence: 'occ-B', occurrence_date: '2026-10-05', occurrence_sequence: 13, date: '2026-10-05', schedule: null, title: 'Occurrence title', status: 'todo' };
  backend.events = [occA, occB];
  return { backend, requests, occA, occB };
}

const cases = [
  ['completed occurrence and Todo neighbor initialize independently', f => {
    f.backend.select('task-1', f.occA);
    assert.equal(f.backend.activeKey, 'task-1:occ-A');
    assert.equal(f.backend.activeDraft.status, 'completed');
    f.backend.select('task-1', f.occB);
    assert.equal(f.backend.activeKey, 'task-1:occ-B');
    assert.equal(f.backend.activeDraft.status, 'todo');
  }],
  ['occurrence save sends status and keeps occurrence key after save', f => {
    f.backend.select('task-1', f.occA);
    f.backend.activeDraft.status = 'todo';
    f.backend.activeDraft.dirty = true;
    f.backend.saveDraft();
    assert.equal(f.requests[0].command, 'occurrence');
    assert.equal(f.requests[0].args.status, 'todo');
    f.requests[0].cb({ id: 'task-1', hash: 'hash-2', revision: 9, title: 'Series title', status: 'todo' }, true);
    assert.equal(f.backend.activeKey, 'task-1:occ-A');
    assert.equal(f.backend.activeDraft.occurrence, 'occ-A');
    assert.equal(f.backend.activeDraft.occurrence_date, '2026-09-28');
    assert.equal(f.backend.activeDraft.occurrence_sequence, 12);
    assert.equal(f.backend.activeDraft.status, 'todo');
    assert.equal(f.backend.activeDraft.dirty, false);
    f.backend.activeKey = '';
    f.backend.select('task-1', { ...f.occA, status: 'todo' });
    assert.equal(f.backend.activeKey, 'task-1:occ-A');
    assert.equal(f.backend.activeDraft.occurrence, 'occ-A');
    assert.equal(f.backend.activeDraft.status, 'todo');
  }],
  ['draft edits made during save survive the response', f => {
    f.backend.select('task-1', f.occA);
    f.backend.activeDraft.title = 'Submitted title';
    f.backend.activeDraft.dirty = true;
    f.backend.saveDraft();
    f.backend.activeDraft.title = 'Concurrent edit';
    f.backend.activeDraft.status = 'in_progress';
    f.backend.activeDraft.dirty = true;
    f.requests[0].cb({ id: 'task-1', hash: 'hash-2', revision: 9, title: 'Series title', status: 'todo' }, true);
    assert.equal(f.backend.activeKey, 'task-1:occ-A');
    assert.equal(f.backend.activeDraft.title, 'Concurrent edit');
    assert.equal(f.backend.activeDraft.status, 'in_progress');
    assert.equal(f.backend.activeDraft.occurrence, 'occ-A');
    assert.equal(f.backend.activeDraft.occurrence_sequence, 12);
    assert.equal(f.backend.activeDraft.hash, 'hash-2');
    assert.equal(f.backend.activeDraft.dirty, true);
  }],
  ['save response does not replace a different active selection', f => {
    f.backend.select('task-1', f.occA);
    f.backend.activeDraft.title = 'Submitted title';
    f.backend.activeDraft.dirty = true;
    f.backend.saveDraft();
    f.backend.select('task-1', f.occB);
    const neighbor = f.backend.activeDraft;
    f.requests[0].cb({ id: 'task-1', hash: 'hash-2', revision: 9, title: 'Series title', status: 'todo' }, true);
    assert.equal(f.backend.activeKey, 'task-1:occ-B');
    assert.equal(f.backend.activeDraft, neighbor);
    assert.equal(f.backend.activeDraft.occurrence, 'occ-B');
    assert.equal(f.backend.activeDraft.status, 'todo');
  }],
  ['clean occurrence reselection refreshes its status from the event', f => {
    f.backend.select('task-1', f.occA);
    f.backend.activeDraft.status = 'todo';
    f.backend.activeDraft.dirty = false;
    f.backend.select('task-1', { ...f.occA, status: 'completed' });
    assert.equal(f.backend.activeDraft.status, 'completed');
  }],
  ['missing occurrence event keeps dirty draft and reports reload unavailable', f => {
    f.backend.select('task-1', f.occA);
    f.backend.activeDraft.title = 'Unsaved title';
    f.backend.activeDraft.status = 'in_progress';
    f.backend.activeDraft.dirty = true;
    f.backend.events = [];
    const key = f.backend.activeKey;
    const draft = JSON.stringify(f.backend.activeDraft);
    f.backend.reloadDraft();
    assert.equal(f.backend.activeKey, key);
    assert.equal(JSON.stringify(f.backend.activeDraft), draft);
    assert.equal(f.backend.activeDraft.dirty, true);
    assert.match(f.backend.messageCalls.at(-1).text, /reload unavailable/i);
    assert.equal(f.backend.messageCalls.at(-1).error, true);
  }],
  ['failed occurrence save leaves draft and key unchanged', f => {
    f.backend.select('task-1', f.occA);
    f.backend.activeDraft.title = 'Unsaved title';
    f.backend.activeDraft.status = 'in_progress';
    f.backend.activeDraft.dirty = true;
    const key = f.backend.activeKey;
    const draft = JSON.stringify(f.backend.activeDraft);
    f.backend.saveDraft();
    assert.equal(f.requests[0].command, 'occurrence');
    f.requests[0].cb(null, false);
    assert.equal(f.backend.activeKey, key);
    assert.equal(JSON.stringify(f.backend.activeDraft), draft);
    assert.equal(f.backend.activeDraft.dirty, true);
    assert.equal(f.backend.saving, false);
  }],
  ['discard reloads the same occurrence using refreshed occurrence values', f => {
    f.backend.select('task-1', f.occA);
    f.backend.activeDraft.title = 'Unsaved title';
    f.backend.activeDraft.status = 'in_progress';
    f.backend.events = [{ ...f.occA, date: '2026-09-29', title: 'Saved occurrence title', status: 'completed' }];
    f.backend.reloadDraft();
    assert.equal(f.backend.activeKey, 'task-1:occ-A');
    assert.equal(f.backend.activeDraft.occurrence, 'occ-A');
    assert.equal(f.backend.activeDraft.occurrence_date, '2026-09-28');
    assert.equal(f.backend.activeDraft.occurrence_sequence, 12);
    assert.equal(f.backend.activeDraft.scheduled_for, '2026-09-29');
    assert.equal(f.backend.activeDraft.title, 'Saved occurrence title');
    assert.equal(f.backend.activeDraft.status, 'completed');
  }]
];

const results = {};
let exit = 0;
for (const [label, path] of [['baseline', process.argv[2]], ['scratch', process.argv[3]]]) {
  const source = fs.readFileSync(path, 'utf8');
  const runs = cases.map(([name, test]) => {
    try { test(fixture(source)); return { name, result: 'PASS' }; }
    catch (error) { return { name, result: 'FAIL', error: String(error.message).split('\n')[0] }; }
  });
  const passed = runs.filter(x => x.result === 'PASS').length;
  results[label] = { passed, total: runs.length, cases: runs };
}
const baselineDetected = results.baseline.passed < results.baseline.total;
const scratchPasses = results.scratch.passed === results.scratch.total;
const report = { baselineDetected, scratchPasses, results };
console.log(JSON.stringify(report, null, 2));
if (process.argv[4]) fs.writeFileSync(process.argv[4], JSON.stringify(report, null, 2) + '\n');
if (!baselineDetected || !scratchPasses) exit = 1;
process.exitCode = exit;
