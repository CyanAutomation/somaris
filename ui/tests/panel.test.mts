import {after, before, test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {chromium, type Browser, type Page} from 'playwright';
import {
  renderBatchControls,
  renderBlindActions,
  renderBlindCard,
  renderDeviceList,
  renderTopbarActions,
} from '../components.mjs';

const styles = [
  'style.css',
  'components.css',
  'panel.css',
].map(file => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8')).join('\n');

let browser: Browser;

before(async () => {
  browser = await chromium.launch({headless:true});
});

after(async () => {
  await browser?.close();
});

function panelMarkup(): string {
  const cards = Array.from({length:4}, (_, index) => {
    const id = `blind-${index + 1}`;
    const label = `Bedroom ${index + 1}`;
    const controls = renderBlindActions({deviceId:id, blindLabel:label, panelMode:true});
    return renderBlindCard({
      id,
      label,
      availabilityLabel:'Observed recently',
      availabilityStatus:'ready',
      positionPercent:50,
      movementStatus:'at_target',
      controlsMarkup:controls,
      panelMode:true,
    });
  }).join('');
  const toolbar = renderBatchControls({blindCount:4, eligibleCount:4, panelMode:true});
  const devices = renderDeviceList({count:4, panelMode:true, cardsMarkup:cards, toolbarMarkup:toolbar});
  const topbar = renderTopbarActions({blindCount:4, panelMode:true, displayPreference:'panel'});

  return `<!doctype html><html lang="en" data-display-mode="panel"><head><meta name="viewport" content="width=device-width, initial-scale=1"><style>${styles}</style></head><body><div class="app-shell"><header class="topbar"><a class="brand" href="#home">somaris.</a><div class="top-actions">${topbar}</div></header><main id="app">${devices}</main></div></body></html>`;
}

async function openPanel(width: number, height: number): Promise<Page> {
  const page = await browser.newPage({viewport:{width, height}});
  await page.setContent(panelMarkup());
  return page;
}

test('panel view lays out blind cards in two columns at 480px with usable controls', async () => {
  // Contract: UI-PANEL-01 in docs/test-contracts.md.
  const page = await openPanel(480, 640);
  try {
    const layout = await page.locator('.device-list--panel .grid').evaluate(grid => {
      const cards = [...grid.children].map(card => {
        const rect = card.getBoundingClientRect();
        return {left:rect.left, width:rect.width};
      });
      const buttons = [...document.querySelectorAll('.card--panel .controls button')]
        .map(button => button.getBoundingClientRect().height);
      const batchButtons = [...document.querySelectorAll('.all-blinds-actions button')]
        .map(button => button.getBoundingClientRect().height);
      return {cards, buttons, batchButtons, scrollWidth:document.documentElement.scrollWidth};
    });

    assert.equal(layout.cards.length, 4);
    assert.equal(layout.cards[0].left, layout.cards[2].left);
    assert.equal(layout.cards[1].left, layout.cards[3].left);
    assert.ok(layout.cards[0].left < layout.cards[1].left);
    assert.ok(layout.cards.every(card => card.width > 0));
    assert.ok(layout.buttons.every(height => height >= 44));
    assert.ok(layout.batchButtons.every(height => height >= 44));
    assert.ok(layout.scrollWidth <= 480);
  } finally {
    await page.close();
  }
});

test('panel view keeps its top bar fixed while blind cards scroll inside the viewport', async () => {
  // Contract: UI-PANEL-02 in docs/test-contracts.md.
  const page = await openPanel(480, 360);
  try {
    const layout = await page.evaluate(() => {
      const topbar = document.querySelector('.topbar')!.getBoundingClientRect();
      const app = document.querySelector('#app')!;
      const shell = document.querySelector('.app-shell')!.getBoundingClientRect();
      const topbarTopBeforeScroll = topbar.top;
      app.scrollTop = 200;
      return {
        topbarTopBeforeScroll,
        topbarTopAfterScroll:document.querySelector('.topbar')!.getBoundingClientRect().top,
        appScrollTop:app.scrollTop,
        appCanScroll:app.scrollHeight > app.clientHeight,
        shellHeight:shell.height,
        viewportHeight:window.innerHeight,
        bodyCanScroll:document.documentElement.scrollHeight > window.innerHeight,
      };
    });

    assert.equal(layout.topbarTopAfterScroll, layout.topbarTopBeforeScroll);
    assert.ok(layout.appScrollTop > 0);
    assert.ok(layout.appCanScroll);
    assert.equal(layout.shellHeight, layout.viewportHeight);
    assert.equal(layout.bodyCanScroll, false);
  } finally {
    await page.close();
  }
});

test('narrow panel view stacks blind cards and batch controls without horizontal overflow', async () => {
  // Contract: UI-PANEL-03 in docs/test-contracts.md.
  const page = await openPanel(359, 640);
  try {
    const layout = await page.evaluate(() => {
      const cards = [...document.querySelectorAll('.device-list--panel .grid > article')]
        .map(card => card.getBoundingClientRect());
      const controls = document.querySelector('.all-blinds-controls')!.getBoundingClientRect();
      const controlsGrid = document.querySelector('.all-blinds-controls')!;
      const copy = document.querySelector('.all-blinds-copy')!.getBoundingClientRect();
      const actions = document.querySelector('.all-blinds-actions')!;
      const actionsRect = actions.getBoundingClientRect();
      const panelButtons = [...document.querySelectorAll('.card--panel .controls button')]
        .map(button => button.getBoundingClientRect().height);
      const actionButtons = [...actions.querySelectorAll('button')]
        .map(button => button.getBoundingClientRect().height);
      return {
        cards,
        controlsWidth:controls.width,
        copyBottom:copy.bottom,
        actionsTop:actionsRect.top,
        controlsColumns:getComputedStyle(controlsGrid).gridTemplateColumns.split(' ').length,
        actionsColumns:getComputedStyle(actions).gridTemplateColumns.split(' ').length,
        panelButtons,
        actionButtons,
        scrollWidth:document.documentElement.scrollWidth,
      };
    });

    assert.equal(layout.cards.length, 4);
    assert.ok(layout.cards.every((card, index) => index === 0 || card.top > layout.cards[index - 1].top));
    assert.ok(layout.controlsWidth > 0);
    assert.ok(layout.copyBottom <= layout.actionsTop);
    assert.equal(layout.controlsColumns, 1);
    assert.equal(layout.actionsColumns, 2);
    assert.ok(layout.panelButtons.every(height => height >= 44));
    assert.ok(layout.actionButtons.every(height => height >= 44));
    assert.ok(layout.scrollWidth <= 359);
  } finally {
    await page.close();
  }
});
