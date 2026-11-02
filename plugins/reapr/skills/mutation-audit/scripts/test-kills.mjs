#!/usr/bin/env node
// Per-test evidence for one Reapr run: which in-scope mutants each test covers and kills.
// Usage: node test-kills.mjs <run_dir>
// Reads <run_dir>/report.json (Reapr's in-scope mutants) and <run_dir>/mutation.json (Stryker's raw
// report, mutation-testing-report-schema v2: coveredBy/killedBy per mutant, testFiles with test ids).
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const runDir = process.argv[2];
if (!runDir) {
  console.error('usage: node test-kills.mjs <run_dir>');
  process.exit(2);
}
const read = (f) => JSON.parse(readFileSync(join(runDir, f), 'utf8'));
const report = read('report.json');
const raw = read('mutation.json');

const inScope = new Set(report.results.map((r) => r.mutant.id));
const tests = new Map(); // id -> { file, name, covers: [], kills: [] }
for (const [file, tf] of Object.entries(raw.testFiles ?? {})) {
  for (const t of tf.tests ?? []) tests.set(t.id, { id: t.id, file, name: t.name, covers: [], kills: [] });
}
const entry = (id) => {
  if (!tests.has(id)) tests.set(id, { id, file: null, name: null, covers: [], kills: [] });
  return tests.get(id);
};

let sawCoverage = false;
for (const [file, f] of Object.entries(raw.files ?? {})) {
  for (const m of f.mutants ?? []) {
    if (!inScope.has(m.id)) continue;
    const where = `${file}:${m.location.start.line}:${m.location.start.column} ${m.mutatorName}`;
    if (m.coveredBy) sawCoverage = true;
    for (const t of m.coveredBy ?? []) entry(t).covers.push(where);
    for (const t of m.killedBy ?? []) entry(t).kills.push(where);
  }
}

const rows = [...tests.values()].filter((t) => t.covers.length || t.kills.length);
const out = {
  per_test_coverage: sawCoverage,
  note: sawCoverage
    ? 'killedBy lists the tests that failed. With bail on (Stryker default), only the first failing test is recorded, so "kills none" is a candidate: confirm by reading the test.'
    : 'No coveredBy data: the engine config does not record per-test coverage (Stryker: coverageAnalysis "perTest"). Find weak tests by reading them instead; do not change the config.',
  tests_covering_but_killing_none: rows.filter((t) => t.covers.length && !t.kills.length),
  tests: rows.map(({ covers, kills, ...t }) => ({ ...t, covers: covers.length, kills: kills.length })),
};
console.log(JSON.stringify(out, null, 2));
