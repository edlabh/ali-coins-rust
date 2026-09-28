#!/usr/bin/env node
// Gera fixtures de cripto a partir do oráculo Node (reference/ali-coins).
// Saída: tools/parity/fixtures/crypto/tokens.json (segredo de teste, sem dados reais).
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const ref = path.join(root, 'reference', 'ali-coins');
const outDir = path.join(root, 'tools', 'parity', 'fixtures', 'crypto');

if (!fs.existsSync(path.join(ref, 'security.js'))) {
  console.error('Oráculo não encontrado em reference/ali-coins.');
  process.exit(1);
}

const require = createRequire(import.meta.url);
const security = require(path.join(ref, 'security.js'));

// Segredo sintético com >= 32 caracteres (nunca usar segredo real em fixture).
const SECRET = 'parity-test-secret-0123456789abcdef';
const PLAINTEXT = JSON.stringify(
  {
    cookies: [{ name: 'xman_us_t', value: 'fake-cookie', domain: '.aliexpress.com' }],
    origins: [],
  },
  null,
  2
);

function encryptV1(payloadJson, secret) {
  const salt = Buffer.from(security.APP_SCRYPT_SALT_V1, 'utf-8');
  const key = crypto.scryptSync(secret, salt, 32, { N: 16384, r: 8, p: 1, maxmem: 64 * 1024 * 1024 });
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  const ciphertext = Buffer.concat([cipher.update(payloadJson, 'utf-8'), cipher.final()]);
  const tag = cipher.getAuthTag();
  key.fill(0);
  return `v1:${iv.toString('base64')}:${tag.toString('base64')}:${ciphertext.toString('base64')}:base64`;
}

const v1 = encryptV1(PLAINTEXT, SECRET);
const v2 = security.encryptSession(PLAINTEXT, SECRET, { version: 'v2' });
const v3 = security.encryptSession(PLAINTEXT, SECRET, { N: 16384, r: 8, p: 1 });

// Token v3 gerado com os defaults efetivos da máquina (registra o N usado).
const defaultN = security.getEffectiveDefaultScryptN();
const v3default = security.encryptSession(PLAINTEXT, SECRET, {});
const v3compact = v3default.split(':').slice(0, 5).concat(['base64']).join(':');

// Round-trip obrigatório: os três precisam decifrar no próprio oráculo.
for (const [name, token] of [
  ['v1', v1],
  ['v2', v2],
  ['v3', v3],
  ['v3default', v3default],
]) {
  const decrypted = security.decryptSession(token, SECRET);
  if (decrypted !== PLAINTEXT) {
    console.error(`Round-trip falhou para ${name}`);
    process.exit(1);
  }
}

const malformed = [
  '',
  'v1:',
  'v2:so:alguns:campos',
  'v3:16384:8:1:apenas:salt',
  'v9:aaaa:bbbb:cccc:dddd:base64',
  `${v3}:lixo-extra-com-dois-pontos`,
  'v3:16384:8:1:!!!!:!!!!:!!!!:!!!!:base64',
];

const payload = {
  generator: 'tools/parity/node/gen_tokens.mjs',
  oracleCommit: process.env.ORACLE_COMMIT || null,
  secret: SECRET,
  plaintext: PLAINTEXT,
  scryptDefaultN: defaultN,
  tokens: { v1, v2, v3, v3default, v3compact },
  malformed,
};

fs.mkdirSync(outDir, { recursive: true });
const outFile = path.join(outDir, 'tokens.json');
fs.writeFileSync(outFile, `${JSON.stringify(payload, null, 2)}\n`, { mode: 0o600 });
console.log(`Fixtures de cripto geradas: ${path.relative(root, outFile)} (N default=${defaultN})`);
