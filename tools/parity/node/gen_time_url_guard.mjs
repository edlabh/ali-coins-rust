#!/usr/bin/env node
// Gera fixtures de tempo (time_utils.js) e de SSRF (libs/url_guard.js).
// Saída: tools/parity/fixtures/common/time.json e url_guard.json
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const ref = path.join(root, 'reference', 'ali-coins');
const outDir = path.join(root, 'tools', 'parity', 'fixtures', 'common');
fs.mkdirSync(outDir, { recursive: true });

// ---- time_utils (fuso default America/Los_Angeles) ----
const timeModulePath = JSON.stringify(path.join(ref, 'time_utils.js'));
const timeCode = `
  const t = require(${timeModulePath});
  const instants = [
    '2026-01-15T12:34:56.000Z',
    '2026-06-15T07:00:00.000Z',
    '2026-03-08T09:59:59.000Z',
    '2026-11-01T08:59:59.000Z'
  ];
  const out = {
    timezone: process.env.REPORT_TIMEZONE || 'America/Los_Angeles',
    cases: instants.map((iso) => {
      const d = new Date(iso);
      return { iso, date: t.formatDate(d), time: t.formatTime(d), dateTime: t.formatDateTime(d), label: t.getReportTimeZoneLabel(d) };
    }),
    durations: [0, 45000, 80000, 3665000, -5, 59_999, 3_600_000].map((ms) => t.formatDuration(ms)),
    backoff: [0, 1, 10].map((a) => t.calculateAccountBackoff(a, null, 30000, 0.5)),
    backoffEnv: (() => { process.env.ACCOUNT_BACKOFF_BASE_MS = '1000'; return t.calculateAccountBackoff(0, null, 30000, 0.5); })(),
    pauses: [[0, 0, 0], [1000, 2000, 0], [1000, 2000, 0.999], [5000, 1000, 0], [0.5, 3.5, 0.5]]
      .map(([mi, ma, r]) => t.pickPauseMs(mi, ma, () => r)),
    compose: [t.composeAccountWaitMs(5000, 1000), t.composeAccountWaitMs(0, 3000), t.composeAccountWaitMs(NaN, -5)]
  };
  console.log(JSON.stringify(out));
`;
const timeRun = spawnSync(process.execPath, ['-e', timeCode], {
  encoding: 'utf-8',
  env: { PATH: process.env.PATH, HOME: process.env.HOME },
});
if (timeRun.status !== 0) {
  console.error(timeRun.stderr);
  process.exit(1);
}
fs.writeFileSync(path.join(outDir, 'time.json'), `${JSON.stringify(JSON.parse(timeRun.stdout), null, 2)}\n`);
console.log('OK   time.json');

// ---- url_guard ----
const require = createRequire(import.meta.url);
const guard = require(path.join(ref, 'libs', 'url_guard.js'));

const ipCorpus = [
  '10.0.0.1', '127.0.0.1', '0.0.0.0', '169.254.169.254', '172.16.0.1', '172.31.255.255',
  '172.32.0.1', '192.168.1.1', '100.64.0.1', '100.128.0.1', '192.0.0.1', '192.0.2.5',
  '192.88.99.1', '198.18.0.1', '198.51.100.7', '203.0.113.9', '224.0.0.1', '255.255.255.255',
  '8.8.8.8', '1.1.1.1', '::1', '::', 'fe80::1', 'febf::1', 'fec0::1', 'fc00::1', 'fd12:3456::1',
  'ff02::1', '::ffff:127.0.0.1', '::ffff:7f00:1', '::7f00:1', '64:ff9b::a9fe:a9fe',
  '2002:a9fe:a9fe::', '2001:0000:4136:e378:8000:63bf:3fff:fdd2', '2606:4700:4700::1111',
  '2001:4860:4860::8888', '2002:0808:0808::', 'nao-e-ip', ''
];

const urlCorpus = [
  'ftp://example.com/x',
  'file:///etc/passwd',
  'nao-e-url',
  '',
  'http://localhost/x',
  'http://localhost./x',
  'http://ip6-localhost/x',
  'http://127.0.0.1/x',
  'http://[::1]/x',
  'http://169.254.169.254/latest/meta-data',
  'http://10.0.0.5/x',
  'http://[::ffff:127.0.0.1]/x',
  'https://example.com/hook',
  'http://8.8.8.8/x',
  'http://user:pass@example.com/x'
];

const payload = {
  generator: 'tools/parity/node/gen_time_url_guard.mjs',
  isPrivateIp: Object.fromEntries(ipCorpus.map((ip) => [ip, guard.isPrivateIp(ip)])),
  validateExternalUrl: {}
};
for (const raw of urlCorpus) {
  // Sem DNS para determinismo: hosts literais/loopsback/erros são síncronos.
  // eslint-disable-next-line no-await-in-loop
  const result = await guard.validateExternalUrl(raw, { resolveDns: false, allowPrivate: false });
  payload.validateExternalUrl[raw] = { ok: result.ok, reason: result.reason ?? null };
}
fs.writeFileSync(path.join(outDir, 'url_guard.json'), `${JSON.stringify(payload, null, 2)}\n`);
console.log('OK   url_guard.json');
