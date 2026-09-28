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
  })),

  buildUnifiedReportPayload: [
    {
      checkin: {
        alreadyCollected: false,
        coinsGainedToday: '25',
        streakDays: 4,
        previousStreakDays: 3,
        totalBalance: '125',
        duration: '12s',
        userEmail: 'u@e.com',
        startTime: '2026-09-28T10:00:00.000Z'
      },
      tasks: {
        results: [{ title: 'Tarefa A', status: 'Concluída (3/3)' }],
        initialBalance: 100,
        finalBalance: 140,
        coinsGained: 40,
        finalCoins: '140',
        duration: '30s',
        endTime: '2026-09-28T10:00:50.000Z'
      },
      meta: {
        mainStartTime: '2026-09-28T10:00:00.000Z',
        mainEndTime: '2026-09-28T10:00:50.000Z'
      }
    },
    { checkin: null, tasks: null, meta: {} },
    {
      checkin: {
        alreadyCollected: true,
        coinsGainedToday: '40',
        streakDays: 212,
        previousStreakDays: 212,
        totalBalance: '900',
        duration: '0s'
      },
      tasks: null,
      meta: { user: 'fulano@example.com', totalDuration: '5s', tasksError: 'boom' }
    }
  ].map(({ checkin, tasks, meta }) => ({
    checkin,
    tasks,
    meta: serializableMeta(meta),
    payload: report.buildUnifiedReportPayload(
      checkin,
      tasks,
      deserializeMeta(meta)
    )
  })),

  buildMultiAccountReportPayload: [
    [
      {
        account: { maskedUser: 'fu***@example.com' },
        checkinResult: {
          alreadyCollected: false,
          coinsGainedToday: '10',
          streakDays: 1,
          totalBalance: '100',
          duration: '10s'
        },
        tasksResult: {
          results: [],
          initialBalance: 100,
          finalBalance: 130,
          coinsGained: 30,
          finalCoins: '130',
          duration: '20s'
        },
        startTime: '2026-09-28T10:00:00.000Z',
        endTime: '2026-09-28T10:00:30.000Z',
        nextAccountAt: '2026-09-28T10:05:00.000Z',
        nextAccountUser: 'outro@example.com'
      },
      {
        user: 'segunda@example.com',
        checkinResult: null,
        tasksResult: null,
        error: 'falha de login',
        isImportedSessionExpired: true
      }
    ],
    []
  ].map((accountResults) => ({
    accountResults: serializableAccounts(accountResults),
    payload: report.buildMultiAccountReportPayload(
      deserializeAccounts(accountResults),
      {}
    )
  })),

  sanitizeWebhookPayload: [
    { user: 'fulano@example.com', sessionData: { a: 1 }, cookies: [1], text: 'ok' },
    { nested: { userEmail: 'beltrano@example.com', arr: ['telefone', '5511999999999'] } },
    { deep: { a: { b: { c: { d: { e: { f: { g: 1 } } } } } } } }
  ].map((value) => ({ value, result: report.sanitizeWebhookPayload(value) }))
};

function serializableMeta(meta) {
  return {
    ...meta,
    mainStartTime: meta.mainStartTime ? new Date(meta.mainStartTime).toISOString() : undefined,
    mainEndTime: meta.mainEndTime ? new Date(meta.mainEndTime).toISOString() : undefined
  };
}

function deserializeMeta(meta) {
  return {
    ...meta,
    mainStartTime: meta.mainStartTime ? new Date(meta.mainStartTime) : undefined,
    mainEndTime: meta.mainEndTime ? new Date(meta.mainEndTime) : undefined
  };
}

function serializableAccounts(accounts) {
  return accounts;
}

function deserializeAccounts(accounts) {
  return accounts;
}

fs.writeFileSync(path.join(outDir, 'report.json'), `${JSON.stringify(payload, null, 2)}\n`);
console.log('OK   report.json');
