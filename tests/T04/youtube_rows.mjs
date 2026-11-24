import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Evaluate the actual QML property expressions with an isolated fixture.
// The frozen T01 expression must fail this distinction to prove the check can
// catch the reported regression; no provider, model or owner job runs.
const root = resolve(import.meta.dirname, '../..');
const base = resolve(root, '../../T01/candidate/ui/YouTubeIntelView.qml');
const candidate = resolve(root, 'ui/YouTubeIntelView.qml');
const fixture = {
  configured: [
    { id: 'source-a', kind: 'youtube-video', location: 'https://www.youtube.com/watch?v=aaaaaaaaaaa' },
    { id: 'source-b', kind: 'youtube-video', location: 'https://www.youtube.com/watch?v=bbbbbbbbbbb' },
  ],
  jobs: [
    { id: 'collection-a', source_id: 'source-a', history_ids: ['history-a'], state: 'completed', result: { title: 'Transcript only' } },
    { id: 'collection-b', source_id: 'source-b', history_ids: ['history-b'], state: 'completed', result: { title: 'Analyzed source' } },
  ],
  archiveRows: [
    { id: 'history-a', provider: 'youtube', title: 'Transcript only' },
    { id: 'history-b', provider: 'youtube', title: 'Analyzed source' },
  ],
  briefingRows: [{ job_id: 'analysis-b', source_id: 'history-b', title: 'Analyzed source', state: 'complete', coverage: { source_segments: 10, selected_segments: 4 } }],
  discoveries: [],
};

function rows(path, property, next) {
  const source = readFileSync(path, 'utf8');
  const start = source.indexOf('property var ' + property + ':');
  assert.ok(start >= 0, property + ' is missing');
  const from = start + ('property var ' + property + ':').length;
  const end = source.indexOf(next, from);
  assert.ok(end > from, property + ' has no bounded expression');
  const expression = source.slice(from, end).trim();
  return Function('root', 'jobs', 'archiveRows', 'briefingRows', 'return (' + expression + ')')(
    fixture, fixture.jobs, fixture.archiveRows, fixture.briefingRows,
  );
}

const baselineResults = rows(base, 'resultRows', 'property var intakeRows:');
assert.deepEqual(baselineResults.map(row => row.history_ids[0]).sort(), ['history-a', 'history-b']);
const results = rows(candidate, 'resultRows', 'property var intakeRows:');
const intake = rows(candidate, 'intakeRows', 'property bool intakeSelected:');
assert.deepEqual(results.map(row => row.history_ids[0]), ['history-b']);
assert.deepEqual(intake.filter(row => row.history_ids?.length).map(row => row.history_ids[0]), ['history-a']);
console.log('T04 YouTube row classification: known-bad T01 detected; transcript-only remains in To analyze; completed analysis enters Results.');
