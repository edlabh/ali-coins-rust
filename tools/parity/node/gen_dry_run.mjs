#!/usr/bin/env node
// Gera fixtures de dry-run a partir do oráculo Node, um por cenário .env.
// Saída: tools/parity/fixtures/config/<cenário>.json  (exit 0) ou .exit (falha).
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const ref = path.join(root, 'reference', 'ali-coins');
const scenariosDir = path.join(root, 'tools', 'parity', 'scenarios');
const outDir = path.join(root, 'tools', 'parity', 'fixtures', 'config');

if (!fs.existsSync(path.join(ref, 'security.js'))) {
  console.error('Oráculo não encontrado em reference/ali-coins.');
  process.exit(1);
}

function parseEnvFile(file) {
  const env = {};
  for (const rawLine of fs.readFileSync(file, 'utf-8').split('\n')) {
    const line = rawLine.trim();
    if (!line || line.startsWith('#')) continue;
    const eq = line.indexOf('=');
    if (eq <= 0) continue;
    const key = line.slice(0, eq).trim();
    let value = line.slice(eq + 1).trim();
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    env[key] = value;
  }
  return env;
}

fs.mkdirSync(outDir, { recursive: true });
let generated = 0;
let failed = 0;

for (const file of fs.readdirSync(scenariosDir).filter((f) => f.endsWith('.env')).sort()) {
  const name = file.replace(/\.env$/, '');
  const scenario = parseEnvFile(path.join(scenariosDir, file));
  const result = spawnSync(process.execPath, ['all.js', '--dry-run', '--json'], {
    cwd: ref,
    env: { ...scenario, PATH: process.env.PATH, HOME: process.env.HOME },
    encoding: 'utf-8',
  });

  const jsonPath = path.join(outDir, `${name}.json`);
  const exitPath = path.join(outDir, `${name}.exit`);
  fs.rmSync(jsonPath, { force: true });
  fs.rmSync(exitPath, { force: true });

  if (result.status === 0) {
    const payload = JSON.parse(result.stdout);
    fs.writeFileSync(jsonPath, `${JSON.stringify(payload, null, 2)}\n`);
    console.log(`OK   ${name} -> fixtures/config/${name}.json`);
  } else {
    fs.writeFileSync(exitPath, `${result.status}\n`);
    console.log(`FALHA ${name} -> exit ${result.status} (fixtures/config/${name}.exit)`);
    failed += 1;
  }
  generated += 1;
}

console.log(`Cenários de dry-run gerados: ${generated} (${failed} com exit != 0).`);
