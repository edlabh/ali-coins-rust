#!/usr/bin/env node
// Gera fixtures de sessão a partir do oráculo Node:
// - storage_filter.json: allowlist + estado filtrado + shouldFilterStorage
// - plain/: session.json + session_meta.json (texto puro)
// - encrypted/: session.json.enc + session_meta.json (v3)
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const ref = path.join(root, 'reference', 'ali-coins');
const outDir = path.join(root, 'tools', 'parity', 'fixtures', 'session');

const require = createRequire(import.meta.url);
const storageFilter = require(path.join(ref, 'libs', 'storage_filter.js'));
const sessionLib = require(path.join(ref, 'libs', 'session.js'));

const SECRET = 'parity-test-secret-0123456789abcdef';
const USER = 'parity-fixture@example.com';

const keys = [
  'userInfo',
  'AUTH_TOKEN',
  '_m_h5_tk',
  'currency',
  'locale',
  'lang',
  'accountId',
  'session_id',
  'loginData',
  'APLUS_S_CORE',
  'aegis',
  'goldlog',
  '',
  'Batman',
];

const input = {
  cookies: [
    { name: 'xman_us_t', value: 'auth-value', domain: '.aliexpress.com' },
    { name: 'login_aliyunid_ticket', value: 'ticket' },
  ],
  origins: [
    {
      origin: 'https://www.aliexpress.com',
      localStorage: [
        { name: 'userInfo', value: '1' },
        { name: 'aegis', value: 'ruido' },
        { name: 'AUTH_TOKEN', value: 't' },
        { name: 'APLUS_S_CORE', value: 'cache' },
      ],
    },
    { origin: 'https://m.aliexpress.com', localStorage: 'invalido' },
    'origem-invalida',
  ],
};

const envCases = {};
for (const value of [undefined, '', 'false', '0', 'off', 'no', 'true', 'yes']) {
  if (value === undefined) delete process.env.SESSION_STRICT_STORAGE;
  else process.env.SESSION_STRICT_STORAGE = value;
  envCases[value === undefined ? '__unset__' : value] = storageFilter.shouldFilterStorage({});
}

const payload = {
  generator: 'tools/parity/node/gen_storage_filter.mjs',
  allowed: Object.fromEntries(keys.map((key) => [key, storageFilter.isAllowedStorageKey(key)])),
  shouldFilterStorageEnv: envCases,
  shouldFilterStorageOptions: {
    true: storageFilter.shouldFilterStorage({ filterStorage: true }),
    false: storageFilter.shouldFilterStorage({ filterStorage: false }),
  },
  input,
  filtered: storageFilter.filterStorageState(input),
};

fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(
  path.join(outDir, 'storage_filter.json'),
  `${JSON.stringify(payload, null, 2)}\n`
);
console.log('OK   storage_filter.json');

// Sessões reais gravadas pelo oráculo (puro e cifrado).
const storageState = {
  cookies: [
    { name: 'xman_us_t', value: 'auth-value', domain: '.aliexpress.com', path: '/' },
    { name: 'ruido', value: 'x' },
  ],
  origins: [
    {
      origin: 'https://www.aliexpress.com',
      localStorage: [
        { name: 'userInfo', value: '1' },
        { name: 'aegis', value: 'ruido' },
      ],
    },
  ],
};

const plainDir = path.join(outDir, 'plain');
const encryptedDir = path.join(outDir, 'encrypted');
for (const dir of [plainDir, encryptedDir]) {
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
}

const plainSaved = await sessionLib.saveSession(storageState, USER, {
  baseDir: plainDir,
  encryptLocalSession: false,
});
if (!plainSaved) throw new Error('saveSession plaintext retornou null');
console.log('OK   plain/session.json + plain/session_meta.json');

const encryptedSaved = await sessionLib.saveSession(storageState, USER, {
  baseDir: encryptedDir,
  secret: SECRET,
  encryptLocalSession: true,
});
if (!encryptedSaved) throw new Error('saveSession encrypted retornou null');
console.log('OK   encrypted/session.json.enc + encrypted/session_meta.json');
