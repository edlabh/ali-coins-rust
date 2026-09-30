#!/usr/bin/env node
// Gera fixtures de snapshot das mensagens do Telegram (libs/notify.js do oráculo).
// Saída: tools/parity/fixtures/common/notify.json
//
// O relógio é congelado para o snapshot ser determinístico; host/usuário/versão
// são fixos e registrados na fixture para o teste em Rust reproduzi-los.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const ref = path.join(root, 'reference', 'ali-coins');
const outDir = path.join(root, 'tools', 'parity', 'fixtures', 'common');
fs.mkdirSync(outDir, { recursive: true });

// Relógio congelado (30/09/2026 15:00:00 UTC = 08:00 em America/Los_Angeles).
const FIXED_NOW = new Date('2026-09-30T15:00:00.000Z');
const RealDate = Date;
globalThis.Date = class extends RealDate {
  constructor(...args) {
    super(...(args.length ? args : [FIXED_NOW.getTime()]));
  }
  static now() {
    return FIXED_NOW.getTime();
  }
};

process.env.NOTIFY_HOST_LABEL = 'vm-ali-rust';
process.env.ALI_USER = 'edelanoali@gmail.com';
process.env.REPORT_TIMEZONE = 'America/Los_Angeles';

const require = createRequire(import.meta.url);
const notify = require(path.join(ref, 'libs', 'notify.js'));
const pkg = require(path.join(ref, 'package.json'));

const HOST = process.env.NOTIFY_HOST_LABEL;
const VERSION = pkg.version;
const USER_MASKED = 'ed***@gmail.com';

const cases = [];
const push = (name, event, options = {}) => {
  const message = notify.buildMessage({
    report: options.report ?? null,
    event,
    error: options.error ?? null,
    customMessage: options.customMessage ?? null,
    hostname: HOST
  });
  cases.push({
    name,
    event,
    error: options.error ?? null,
    // Sem report o resolveUser do oráculo devolve null (nenhuma linha de conta).
    user: options.report ? (options.user ?? USER_MASKED) : null,
    previousStreakDays: options.previousStreakDays ?? null,
    streakDays: options.streakDays ?? null,
    totalBalance: options.totalBalance ?? null,
    message
  });
};

push('dry_run', 'dry_run');
push('manual_test', 'manual_test');
push('lock_active_com_erro', 'lock_active', {
  error: 'Outra instância mantém o lock para ed***@gmail.com.'
});
push('lock_active_sem_erro', 'lock_active');
push('failure_simples', 'failure', { error: 'Falha na execução: Request timed out' });
push('failure_com_segredo', 'failure', {
  error: 'GET https://x/y?token=abc123&session=zzz falhou'
});
push('streak_break', 'streak_break', {
  report: { checkin: { previousStreakDays: 226, streakDays: 1, totalBalance: '150' } },
  previousStreakDays: 226,
  streakDays: 1,
  totalBalance: '150 moedas'
});
push('2fa_required', '2fa_required');
push('captcha_required', 'captcha_required', {
  error: 'Captcha solicitado durante o login (code=abcd)'
});
push('captcha_cooldown_released', 'captcha_cooldown_released');

const payload = {
  generator: 'tools/parity/node/gen_notify.mjs',
  now: FIXED_NOW.toISOString(),
  host: HOST,
  version: VERSION,
  cases
};

fs.writeFileSync(
  path.join(outDir, 'notify.json'),
  `${JSON.stringify(payload, null, 2)}\n`,
  'utf8'
);
console.log(`notify.json: ${cases.length} casos gerados`);
