#!/usr/bin/env node
// Gera a fixture do perfil mobile a partir do Playwright instalado no oráculo.
// Saída: tools/parity/fixtures/common/device_pixel7.json
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const ref = path.join(root, 'reference', 'ali-coins');
const outDir = path.join(root, 'tools', 'parity', 'fixtures', 'common');

const require = createRequire(path.join(ref, 'package.json'));
const { devices } = require('playwright');
const device = devices['Pixel 7'];
if (!device) {
  console.error('Playwright sem o device Pixel 7.');
  process.exit(1);
}

const payload = {
  generator: 'tools/parity/node/gen_device.mjs',
  userAgent: device.userAgent,
  viewport: { width: device.viewport.width, height: device.viewport.height },
  deviceScaleFactor: device.deviceScaleFactor,
  isMobile: device.isMobile,
  hasTouch: device.hasTouch,
  locale: 'pt-BR',
};

fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(
  path.join(outDir, 'device_pixel7.json'),
  `${JSON.stringify(payload, null, 2)}\n`
);
console.log('OK   device_pixel7.json');
