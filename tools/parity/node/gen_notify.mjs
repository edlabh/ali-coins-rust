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

// Casos multi-conta (payload montado pelo builder do oráculo).
const report = require(path.join(ref, 'libs', 'report.js'));
const accountResults = [
  {
    account: { maskedUser: 'conta1***@gmail.com' },
    checkinResult: {
      alreadyCollected: false,
      coinsGainedToday: '40',
      streakDays: 226,
      totalBalance: '150',
      duration: '1m 20s'
    },
    tasksResult: { coinsGained: 46, finalCoins: '196 moedas', duration: '5m 10s' },
    startTime: '2026-09-30T14:50:00.000Z',
    endTime: '2026-09-30T14:56:30.000Z'
  },
  {
    account: { maskedUser: 'conta2***@gmail.com' },
    checkinResult: {
      alreadyCollected: true,
      streakDays: 12,
      totalBalance: '88',
      duration: '1m 02s'
    },
    tasksResult: { coinsGained: 0, finalCoins: '88 moedas', duration: '2m 00s' },
    startTime: '2026-09-30T14:57:00.000Z',
    endTime: '2026-09-30T15:00:00.000Z',
    nextAccountAt: null
  }
];
const multiMeta = {
  mainStartTime: new Date('2026-09-30T14:50:00.000Z'),
  mainEndTime: new Date('2026-09-30T15:00:00.000Z'),
  totalDuration: '10m 00s'
};
const multiSuccessReport = report.buildMultiAccountReportPayload(accountResults, multiMeta);
cases.push({
  name: 'multi_success',
  event: 'success',
  error: null,
  report: multiSuccessReport,
  message: notify.buildMessage({ report: multiSuccessReport, event: 'success', hostname: HOST })
});
const multiFailureReport = report.buildMultiAccountReportPayload(
  [
    accountResults[0],
    { account: { maskedUser: 'conta2***@gmail.com' }, error: 'Falha na navegação: timeout' }
  ],
  multiMeta
);
cases.push({
  name: 'multi_failure',
  event: 'failure',
  error: 'Falha na navegação: timeout',
  report: multiFailureReport,
  message: notify.buildMessage({
    report: multiFailureReport,
    event: 'failure',
    error: 'Falha na navegação: timeout',
    hostname: HOST
  })
});
const multiAlreadyReport = report.buildMultiAccountReportPayload(
  [
    {
      account: { maskedUser: 'conta1***@gmail.com' },
      checkinResult: { alreadyCollected: true, streakDays: 226, totalBalance: '150', duration: '1m 20s' },
      tasksResult: { coinsGained: 0, finalCoins: '150 moedas', duration: '2m 00s' },
      startTime: '2026-09-30T14:50:00.000Z',
      endTime: '2026-09-30T14:53:20.000Z'
    }
  ],
  multiMeta
);
cases.push({
  name: 'multi_already_collected',
  event: 'already_collected',
  error: null,
  report: multiAlreadyReport,
  message: notify.buildMessage({ report: multiAlreadyReport, event: 'already_collected', hostname: HOST })
});

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
