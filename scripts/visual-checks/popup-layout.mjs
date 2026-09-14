#!/usr/bin/env node
/**
 * Layout check for the classify popup (人物模式), which jsdom cannot verify.
 *
 * It answers two questions the unit tests cannot:
 *   1. is the character roster still reachable at every window size?
 *   2. does framing (框选主脸) hand the picture almost the whole window?
 *
 * Prerequisites:
 *   - a dev server for the frontend on $APP_URL (default http://127.0.0.1:1437)
 *   - Playwright with a Chromium build available:
 *       PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs \
 *       CHROME_PATH=/path/to/chrome-linux/chrome \
 *       node scripts/visual-checks/popup-layout.mjs
 *
 * Run it after touching PopupClassify.vue layout rules or the framing state.
 */
import process from 'node:process';

const playwrightModule = process.env.PLAYWRIGHT_MODULE;
const executablePath = process.env.CHROME_PATH;
const appUrl = process.env.APP_URL ?? 'http://127.0.0.1:1437';
if (!playwrightModule || !executablePath) {
  console.error('Set PLAYWRIGHT_MODULE and CHROME_PATH first (see the header of this file).');
  process.exit(2);
}

const { chromium } = await import(playwrightModule);
const browser = await chromium.launch({ executablePath, headless: true, args: ['--no-sandbox'] });
const failures = [];

const characters = Array.from({ length: 12 }, (_, index) => ({
  id: `char-${index + 1}`,
  projectId: 'project-1',
  name: `角色 ${String.fromCharCode(65 + index)}`,
  aliasesJson: '[]',
  avatarAssetId: null,
  createdAt: '2026-09-01T00:00:00Z',
  updatedAt: '2026-09-01T00:00:00Z',
}));
const item = {
  id: 'item-1',
  projectId: 'project-1',
  sessionId: 'session-1',
  assetId: null,
  characterId: null,
  classification: 'unclassified',
  sourcePath: 'D:\\Shots\\sample.png',
  annotatedPath: null,
  avatarPath: null,
  destinationPath: null,
  destinationAvatarPath: null,
  status: 'awaiting_label',
  faceBoxJson: '{"x":420,"y":210,"width":240,"height":240}',
  errorMessage: null,
  failureStage: null,
  attemptCount: 0,
  nextRetryAt: null,
  processingWarningsJson: '[]',
  suggestedCharacterId: 'char-2',
  recognitionConfidence: 0.62,
  recognitionSource: 'face_bank',
  reviewStatus: 'pending',
  verificationScore: null,
  verificationStatus: 'unverified',
  bestOtherScore: null,
  bestOtherCharacterId: null,
  faceCount: 2,
  capturedAt: '2026-09-14T00:00:00Z',
  processedAt: null,
  archivedAt: null,
  createdAt: '2026-09-14T00:00:00Z',
  updatedAt: '2026-09-14T00:00:00Z',
  processingVersion: 0,
  manualFaceRoiJson: null,
  manualFaceRoiReady: 0,
  sourceFileState: 'available',
  destinationFileState: 'none',
  destinationAvatarFileState: 'none',
};

async function openPopup(viewport) {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  await page.addInitScript(
    ({ item, characters }) => {
      localStorage.setItem('scene-vault-theme', 'dark');
      localStorage.setItem('scene-vault.capture.project', 'project-1');
      const callbacks = new Map();
      let index = 0;
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: 'classify-popup' }, currentWebview: { label: 'classify-popup' } },
        transformCallback: (fn) => {
          callbacks.set(++index, fn);
          return index;
        },
        invoke: async (command) => {
          switch (command) {
            case 'get_database_startup_status':
              return { mode: 'normal' };
            case 'classify_popup_context':
              return { items: [item], projectId: 'project-1', projectName: '视觉检查', characters };
            case 'get_capture_face_roi':
              return item.manualFaceRoiJson ? JSON.parse(item.manualFaceRoiJson) : null;
            case 'suggest_for_capture':
              return item;
            case 'get_capture_runtime_status':
              return { engineStatus: 'configured', workerStatus: 'idle', queuedCount: 0, prelabelPendingCount: 0, archivePendingCount: 0 };
            case 'get_app_settings':
              return { autoCloseEmptyPopup: false, splitPopupWindows: true };
            case 'read_capture_image':
            case 'read_capture_thumbnail': {
              const canvas = document.createElement('canvas');
              canvas.width = 1200;
              canvas.height = 700;
              const ctx = canvas.getContext('2d');
              const gradient = ctx.createLinearGradient(0, 0, 1200, 700);
              gradient.addColorStop(0, '#203842');
              gradient.addColorStop(1, '#09171f');
              ctx.fillStyle = gradient;
              ctx.fillRect(0, 0, 1200, 700);
              for (const [x, color, name] of [[430, '#578f9a', 'A'], [830, '#957664', 'B']]) {
                ctx.fillStyle = color;
                ctx.beginPath();
                ctx.ellipse(x, 330, 95, 130, 0, 0, Math.PI * 2);
                ctx.fill();
                ctx.fillRect(x - 130, 455, 260, 245);
                ctx.fillStyle = '#d8e7e9';
                ctx.font = '32px sans-serif';
                ctx.fillText(name, x - 10, 140);
              }
              return await (await fetch(canvas.toDataURL('image/png'))).arrayBuffer();
            }
            case 'list_projects':
              return [{ id: 'project-1', name: '视觉检查', createdAt: '2026-09-01T00:00:00Z', updatedAt: '2026-09-01T00:00:00Z' }];
            case 'list_characters':
              return characters;
            case 'record_client_event':
            case 'plugin:event|unlisten':
              return null;
            case 'plugin:event|listen':
              return ++index;
            default:
              return command.startsWith('list_') ? [] : null;
          }
        },
      };
    },
    { item, characters },
  );
  return { page, context, errors };
}

/** Reads the layout without printing, so a check can measure twice. */
async function readLayout(page) {
  return page.evaluate(() => {
    const rect = (element) => (element ? Math.round(element.getBoundingClientRect().height) : null);
    const list = document.querySelector('.popup-characters');
    const rows = [...document.querySelectorAll('.popup-character')];
    return {
      viewport: `${innerWidth}x${innerHeight}`,
      preview: rect(document.querySelector('.popup-preview')),
      list: rect(list),
      rows: rows.length,
      rowsInsideList: list
        ? rows.filter((row) => {
            const box = row.getBoundingClientRect();
            const bounds = list.getBoundingClientRect();
            return box.height > 8 && box.bottom <= Math.min(innerHeight, bounds.bottom) + 1 && box.top >= bounds.top - 1;
          }).length
        : 0,
    };
  });
}

/**
 * Rows the user can actually see. A very short window may have to scroll the
 * body, so the check scrolls once before giving up.
 */
async function visibleRows(page, minRows) {
  const first = await readLayout(page);
  if (first.rowsInsideList >= minRows) return first;
  await page.evaluate(() => {
    const body = document.querySelector('.popup-body--fill');
    if (body) body.scrollTop = body.scrollHeight;
  });
  await page.waitForTimeout(150);
  return readLayout(page);
}

/** The contract every window size has to satisfy. */
const VIEWPORTS = [
  { width: 1115, height: 940 },
  { width: 880, height: 620 },
  { width: 660, height: 640 },
  { width: 560, height: 480 },
];
const MIN_ROWS = 2;
/** Framing leaves exactly the 48px title bar and the hint strip behind. */
const FRAMING_CHROME = 140;

for (const viewport of VIEWPORTS) {
  const { page, context, errors } = await openPopup(viewport);
  await page.goto(`${appUrl}/?popup=workbench`);
  await page.locator('button', { hasText: '人物' }).first().click();
  await page.locator('.popup-characters').waitFor({ state: 'attached' });
  await page.waitForTimeout(300);

  const label = `${viewport.width}x${viewport.height}`;
  const roster = await visibleRows(page, MIN_ROWS);
  console.log(`roster ${label}`, JSON.stringify(roster));
  if (roster.rowsInsideList < MIN_ROWS) {
    failures.push(`roster shows ${roster.rowsInsideList} row(s) at ${label}, expected ${MIN_ROWS}: ${JSON.stringify(roster)}`);
  }
  if ((roster.list ?? 0) < 120) {
    failures.push(`roster list is only ${roster.list}px at ${label}`);
  }

  await page.locator('button', { hasText: '框选主脸' }).first().click();
  await page.waitForTimeout(300);
  const framing = await readLayout(page);
  console.log(`framing ${label}`, JSON.stringify(framing));
  if ((framing.preview ?? 0) < viewport.height - FRAMING_CHROME || (framing.preview ?? 0) < viewport.height * 0.7) {
    failures.push(`framing preview is ${framing.preview}px at ${label}, expected at least ${viewport.height - FRAMING_CHROME}px`);
  }
  if ((framing.list ?? 0) > 0 || framing.rowsInsideList > 0) {
    failures.push(`the roster must be hidden while framing at ${label}: ${JSON.stringify(framing)}`);
  }

  await page.locator('button', { hasText: '取消' }).first().click();
  await page.waitForTimeout(300);
  const restored = await visibleRows(page, MIN_ROWS);
  console.log(`restored ${label}`, JSON.stringify(restored));
  if (restored.rowsInsideList < MIN_ROWS) {
    failures.push(`roster did not come back at ${label}: ${JSON.stringify(restored)}`);
  }

  if (errors.length) failures.push(`page errors at ${viewport.width}x${viewport.height}: ${errors.join(' | ')}`);
  await context.close();
}

await browser.close();
if (failures.length) {
  console.error('FAILED');
  for (const failure of failures) console.error(' -', failure);
  process.exit(1);
}
console.log('POPUP LAYOUT OK');
