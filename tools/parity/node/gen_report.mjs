#!/usr/bin/env node
// Gera fixtures das funções puras de relatório (libs/report.js do oráculo).
// Saída: tools/parity/fixtures/common/report.json
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
const report = require(path.join(ref, 'libs', 'report.js'));

const payload = {
  generator: 'tools/parity/node/gen_report.mjs',
  isStreakBreak: [
    [1, 10, false],
    [7, 212, false],
    [1, 10, true],
    [1, 1, false],
    [null, 10, false],
    [2, 10, false],
    [10, 10, false]
  ].map(([current, previous, already]) => ({
    current,
    previous,
    already,
    result: report.isStreakBreak(current, previous, already)
  })),

  resolveStreakDays: [
    { detectedStreak: 5, previousStreakDays: 5, justCollected: true },
    { detectedStreak: 5, previousStreakDays: 5, alreadyCollected: true },
    { detectedStreak: null, previousStreakDays: 5, justCollected: true },
    { detectedStreak: 7, previousStreakDays: 212, justCollected: true },
    { detectedStreak: 7, previousStreakDays: 212, alreadyCollected: true },
    { detectedStreak: 1, previousStreakDays: 50, statementStreak: 50 },
    { detectedStreak: 60, previousStreakDays: 50 },
    { detectedStreak: 'N/D', previousStreakDays: 50 },
    { detectedStreak: 3, justCollected: true },
    { detectedStreak: '7 dias', previousStreakDays: 7, confirmedByLedger: true }
  ].map((params) => {
    const result = report.resolveStreakDays(params);
    return { params, streakDays: result.streakDays, baseStreak: result.baseStreak };
  }),

  computeCheckinCoinsGained: [
    null,
    { alreadyCollected: true, coinsGainedToday: '40' },
    { alreadyCollected: true, coinsGainedToday: '25', checkinCoinsFromLedger: true },
    { alreadyCollected: false, coinsGainedToday: '0' },
    { alreadyCollected: false, coinsGainedToday: '20' },
    { alreadyCollected: false, coinsGainedToday: 'N/D', streakDays: 4 },
    { alreadyCollected: false, coinsGainedToday: '', streakDays: 'N/D' },
    { alreadyCollected: false },
    { alreadyCollected: true, checkinCoinsFromLedger: true, coinsGainedToday: '' }
  ].map((checkin) => ({
    checkin,
    result: report.computeCheckinCoinsGained(checkin)
  })),

  computeTasksCoinsGained: [
    { tasks: null, checkin: null },
    { tasks: { coinsGained: 30 }, checkin: null },
    { tasks: { coinsGained: 30, coinsFromLedger: true }, checkin: { alreadyCollected: true, coinsGainedToday: '20' } },
    { tasks: { coinsGained: 30, initialBalance: 100 }, checkin: { alreadyCollected: false, coinsGainedToday: '20', totalBalance: '120' } },
    { tasks: { coinsGained: 30, initialBalance: 120 }, checkin: { alreadyCollected: false, coinsGainedToday: '20', totalBalance: '120' } },
    { tasks: { coinsGained: 10, initialBalance: 100 }, checkin: { alreadyCollected: false, coinsGainedToday: '20', totalBalance: '120', balanceBeforeCheckin: true } },
    { tasks: { coinsGained: '30' }, checkin: null },
    { tasks: { coinsGained: 5, initialBalance: '0' }, checkin: { alreadyCollected: false, coinsGainedToday: '20', totalBalance: '20' } }
  ].map(({ tasks, checkin }) => ({
    tasks,
    checkin,
    result: report.computeTasksCoinsGained(tasks, checkin)
  })),

  computeFinalBalance: [
    { checkin: null, tasks: null },
    { checkin: { totalBalance: '99' }, tasks: null },
    { checkin: { totalBalance: 'N/D' }, tasks: { finalCoins: '150' } },
    { checkin: { totalBalance: '99' }, tasks: { finalCoins: 'N/D' } },
    { checkin: null, tasks: { finalCoins: '0' } }
  ].map(({ checkin, tasks }) => ({
    checkin,
    tasks,
    result: report.computeFinalBalance(checkin, tasks)
  }))
};

fs.writeFileSync(path.join(outDir, 'report.json'), `${JSON.stringify(payload, null, 2)}\n`);
console.log('OK   report.json');
