#!/usr/bin/env node
// Normaliza JSON para comparação Node × Rust (stdin → stdout).
// Substitui valores voláteis (tempos, durações, caminhos) por placeholders.
import fs from 'node:fs';

const SENSITIVE_KEYS = new Set([
  'startTime',
  'endTime',
  'savedAt',
  'importedAt',
  'exportedAt',
  'migratedAt',
  'lastRotatedAt',
  'lastCheckinDate',
  'nextAccountAt',
  'generatedAt',
  'timestamp',
]);

const DURATION_KEYS = new Set([
  'duration',
  'totalDuration',
  'step1Duration',
  'step2Duration',
  'durationMs',
]);

function normalize(value, key = '') {
  if (value === null || value === undefined) return value;
  if (SENSITIVE_KEYS.has(key)) return 'T';
  if (DURATION_KEYS.has(key) && typeof value === 'number') return 'T';
  if (Array.isArray(value)) return value.map((item) => normalize(item));
  if (typeof value === 'object') {
    const out = {};
    for (const [k, v] of Object.entries(value)) out[k] = normalize(v, k);
    return out;
  }
  if (typeof value === 'string') {
    const root = process.env.PARITY_ROOT;
    if (root && value.includes(root)) return value.split(root).join('<ROOT>');
  }
  return value;
}

const raw = fs.readFileSync(0, 'utf-8').trim();
if (raw === '') process.stdout.write('\n');
else process.stdout.write(`${JSON.stringify(normalize(JSON.parse(raw)), null, 2)}\n`);
