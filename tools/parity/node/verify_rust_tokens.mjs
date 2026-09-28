#!/usr/bin/env node
// Verifica tokens gerados pelo Rust contra o oráculo Node (deep-equal do payload).
// Uso: cargo run -p ali-coins-core --example gen-tokens | node verify_rust_tokens.mjs
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const oraclePath = path.join(root, 'reference', 'ali-coins', 'security.js');

if (!fs.existsSync(oraclePath)) {
  console.error('Oráculo não encontrado em reference/ali-coins.');
  process.exit(1);
}

const require = createRequire(import.meta.url);
const security = require(oraclePath);

const input = JSON.parse(fs.readFileSync(0, 'utf-8'));
const expected = JSON.parse(input.plaintext);
let failures = 0;

for (const [name, token] of Object.entries(input.tokens)) {
  try {
    const decrypted = security.decryptSession(token, input.secret);
    const actual = JSON.parse(decrypted);
    const ok = JSON.stringify(actual) === JSON.stringify(expected);
    console.log(`${ok ? 'OK  ' : 'FALHA'} ${name}`);
    if (!ok) failures += 1;
  } catch (error) {
    console.error(`ERRO  ${name}: ${error.message}`);
    failures += 1;
  }
}

if (failures > 0) {
  console.error(`${failures} token(s) gerado(s) pelo Rust falharam no oráculo.`);
  process.exit(1);
}
console.log('Todos os tokens Rust foram decifrados pelo oráculo Node.');
