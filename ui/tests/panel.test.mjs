import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const panelCss = readFileSync(new URL('../panel.css', import.meta.url), 'utf8');

test('480px panel layout keeps blind cards in two columns with usable controls', () => {
  assert.match(panelCss, /\.device-list--panel \.grid\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/s);
  assert.match(panelCss, /\.card--panel \.controls \.button\.icon-action\s*\{[^}]*min-height:\s*60px/s);
  assert.match(panelCss, /@media\s*\(max-width:\s*359px\)/);
  assert.doesNotMatch(panelCss, /@media\s*\(max-width:\s*480px\)/);
});

test('panel topbar and cards use the enlarged vertical budget', () => {
  assert.match(panelCss, /\.topbar\s*\{[^}]*height:\s*60px;[^}]*min-height:\s*60px/s);
  assert.match(panelCss, /\.card--panel\s*\{[^}]*min-height:\s*180px/s);
  assert.match(panelCss, /\.card--panel\s*\{[^}]*padding:\s*14px/s);
  assert.doesNotMatch(panelCss, /\.device-list-heading/);
});
