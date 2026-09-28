#!/usr/bin/env node
// Gera fixtures dos parsers de saldo/streak (libs/ui/balance.js).
// Saída: tools/parity/fixtures/common/balance.json
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const ref = path.join(root, 'reference', 'ali-coins');
const outDir = path.join(root, 'tools', 'parity', 'fixtures', 'common');
fs.mkdirSync(outDir, { recursive: true });

const require = createRequire(import.meta.url);
const balance = require(path.join(ref, 'libs', 'ui', 'balance.js'));

const streakTexts = [
  'Sequência de 42 dias',
  'Você está há 15 dias de sequência',
  'You are on a 15-day streak',
  '212 dias consecutivos',
  '7 days in a row',
  'Check-in diário: 7 dias',
  '7 dias de check-in',
  'completou 30 dias',
  'coletou por 3 dias',
  'sem numero nenhum',
  ''
];

const loginTexts = [
  'Sign in with email code',
  'Switch account',
  'Trocar de conta',
  'Forgot password? Digite sua senha',
  'Esqueci minha senha — password',
  'Minhas moedas 120',
  ''
];

const ledgerTexts = [
  '21/09/2026 App daily check-in\n+40\nCoin page task\n+5\nMissões de moedas\n+1.000',
  'Bônus diário\n+20',
  'Daily bonus\n+15\nWidget coins\n+25',
  'Check-in diário no app\n+10',
  'Sem lançamentos',
  'Missões de moedas\n+1,000\nApp daily check-in\n+35'
];

const payload = {
  generator: 'tools/parity/node/gen_balance.mjs',
  extractStreakFromText: Object.fromEntries(
    streakTexts.map((text) => [text, balance.extractStreakFromText(text)])
  ),
  isLoginPromptText: Object.fromEntries(
    loginTexts.map((text) => [text, balance.isLoginPromptText(text)])
  ),
  getStreakFromCheckinCoins: ['+10 moedas', '40', '12', '25', 'N/D'].map((coins) => ({
    coins,
    result: balance.getStreakFromCheckinCoins(coins)
  })),
  getCheckinCoinsFromStreak: [null, 'N/D', 1, 2, 4, 7, 212, '42 dias'].map((streak) => ({
    streak,
    result: balance.getCheckinCoinsFromStreak(streak)
  })),
  extractTodayLedger: Object.fromEntries(
    ledgerTexts.map((text) => [text, balance.extractTodayLedger(text)])
  )
};

fs.writeFileSync(path.join(outDir, 'balance.json'), `${JSON.stringify(payload, null, 2)}\n`);
console.log('OK   balance.json');
