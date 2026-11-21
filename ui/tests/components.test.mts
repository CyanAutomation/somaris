import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseHTML} from 'linkedom';
import {readFileSync} from 'node:fs';
import {
  blindState,
  movementStatusFromPositions,
  renderBlindActions,
  renderBlindCard,
  renderCalibrationControls,
  renderBlindStatus,
  renderPanelNotice,
  renderMotorSpeedControl,
  renderBadge,
  renderStatusIndicator,
  renderDeviceCardHeader,
  renderFreshnessIndicator,
  renderButton,
  renderIconButton,
  renderIconLinkButton,
  renderAddBlindButton,
  renderRoomPicker,
  renderRoomGroup,
  renderRoomCollection,
  renderThemePreferenceControl,
  renderDemoBanner,
  renderPositionPreset,
  renderServiceSummary,
  renderConfirmationPanel,
  renderEmptyState,
  renderAddDeviceEmptyState,
  renderHomeContent,
  renderDisclosure,
  renderDeviceList,
  renderMobileTabs,
  renderBatchControls,
  renderBatchFeedback,
  renderSnackbar,
  renderSnackbarRegion,
  renderTopbarActions,
  renderHomeSupportingSections,
  renderInfoGrid,
  renderBatteryReading,
  renderRelativeTime,
  renderActivitySection,
  renderServiceHealthSummary,
} from '../components.mjs';

function parseMarkup(markup: string) {
  return parseHTML(`<!doctype html><html><body><main id="fixture">${markup}</main></body></html>`)
    .document.querySelector('#fixture')!;
}

test('Somaris keeps the original orange stacked-bar brand glyph', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  const styles = readFileSync(new URL('../style.css', import.meta.url), 'utf8');
  const mark = html.match(/<span class="brand-mark"[^>]*>[\s\S]*?<\/span>/)?.[0] || '';

  assert.equal((mark.match(/<i><\/i>/g) || []).length, 3);
  assert.doesNotMatch(mark, /cord/);
  const brand = html.match(/<div class="topbar-brand">([\s\S]*?)<\/div>/)?.[1] || '';
  assert.doesNotMatch(brand, /header-clock-slot/);
  assert.match(html, /<div id="header-clock-slot" class="topbar-clock"><\/div>/);
  assert.match(styles, /\.brand-mark\{width:32px;height:32px;background:var\(--color-accent\);border-radius:10px;/);
  assert.match(styles, /\.brand-mark i\{height:3px;border-radius:3px;background:var\(--token-brand-ink\)\}/);
});

test('blind state prioritizes reported movement and otherwise describes current position', () => {
  assert.deepEqual(blindState(100, 'opening'), {label:'Opening…', percentOpen:0});
  assert.deepEqual(blindState(0, 'closing'), {label:'Closing…', percentOpen:100});
  assert.deepEqual(blindState(0, 'at_target'), {label:'Open', percentOpen:100});
  assert.deepEqual(blindState(100, 'at_target'), {label:'Closed', percentOpen:0});
  assert.deepEqual(blindState(42, 'at_target'), {label:'Partially open', percentOpen:58});
  assert.deepEqual(blindState(null, 'unavailable'), {label:'Position unavailable', percentOpen:null});
  assert.deepEqual(blindState(101, 'at_target'), {label:'Position unavailable', percentOpen:null});
});

test('movement direction follows the unchanged current and target closed percentages', () => {
  assert.equal(movementStatusFromPositions(20, 80), 'closing');
  assert.equal(movementStatusFromPositions(80, 20), 'opening');
  assert.equal(movementStatusFromPositions(40, 40), 'at_target');
  assert.equal(movementStatusFromPositions(null, 40), 'unavailable');
});

test('blind status renders the state and percentage-open progress', () => {
  const markup = renderBlindStatus({positionPercent:42, movementStatus:'at_target'});
  assert.match(markup, /blind-state__label">Partially open/);
  assert.match(markup, /blind-state__percent">58% open/);
  assert.ok(markup.indexOf('Partially open') < markup.indexOf('58% open'));
  assert.match(renderBlindStatus({positionPercent:40,movementStatus:'opening'}),/Opening… 60%/);
  assert.match(renderBlindStatus({positionPercent:40,movementStatus:'opening'}),/role="progressbar"[^>]*aria-valuenow="60"/);
  assert.doesNotMatch(renderBlindStatus({positionPercent:null, movementStatus:'unavailable'}), /% closed/);
  assert.match(renderBlindStatus({positionPercent:50,movementStatus:'stop_queued'}), /Stop queued/);
  assert.match(renderBlindStatus({positionPercent:50,movementStatus:'stopping'}), /Stopping/);
});

test('shared empty states explain demo enrollment and reuse icon actions', () => {
  const empty = parseMarkup(renderEmptyState({
    title:'No blinds yet', description:'Add a nearby blind.', icon:'home',
    actionMarkup:renderIconButton({label:'Add a blind', action:'add', icon:'add'}),
  }));
  assert.equal(empty.querySelector('.empty h3')?.textContent, 'No blinds yet');
  assert.equal(empty.querySelector('.empty p')?.textContent, 'Add a nearby blind.');
  assert.equal(empty.querySelector('[data-action="add"]')?.getAttribute('aria-label'), 'Add a blind');

  const demo = parseMarkup(renderAddDeviceEmptyState({demoMode:true, adapterAvailable:false}));
  assert.match(demo.textContent || '', /four simulated blinds/i);
  assert.match(demo.textContent || '', /adding bluetooth devices is unavailable/i);
  assert.equal(demo.querySelector('[data-action="refresh"]'), null);
  assert.doesNotMatch(demo.textContent || '', /bluetooth adapter is unavailable/i);

  const normal = parseMarkup(renderAddDeviceEmptyState({demoMode:false, adapterAvailable:false}));
  assert.match(normal.textContent || '', /bluetooth adapter is unavailable/i);
  assert.equal(normal.querySelector('[data-action="refresh"]')?.getAttribute('aria-label'), 'Refresh devices');
});

test('home composition puts the demo guidance before the blinds and omits the duplicated hero', () => {
  const root = parseMarkup(renderHomeContent({
    deviceListMarkup:'<section id="devices"></section>',
    demoHintMarkup:'<aside id="hint"></aside>',
    supportingMarkup:'<section id="supporting"></section>',
  }));
  assert.deepEqual([...root.children].map(child => child.id), ['hint','devices','supporting']);
});

test('icon-only blind actions have named controls, visible glyphs and disabled states', () => {
  const actions = renderBlindActions({deviceId:'blind-1', blindLabel:'Bedroom Left', panelMode:true, disabled:false});
  assert.match(actions, /role="group" aria-label="Bedroom Left controls"/);
  assert.match(actions, /aria-label="Open Bedroom Left"/);
  assert.match(actions, /title="Open Bedroom Left"/);
  assert.match(actions, /data-tooltip="Open Bedroom Left"/);
  assert.match(actions, /data-action="command:open:blind-1"/);
  assert.match(actions, /data-icon="open"/);
  assert.match(actions, /aria-label="Stop Bedroom Left"/);
  assert.match(actions, /aria-label="Close Bedroom Left"/);
  assert.doesNotMatch(actions, /<span class="button-label">Open<\/span>/);

  const disabled = renderBlindActions({deviceId:'blind-1', blindLabel:'Bedroom Left', panelMode:true, disabled:true});
  assert.equal((disabled.match(/ disabled/g) || []).length, 3);
});

test('movement cooldown disables Open and Close while leaving Stop available', () => {
  const controls = renderBlindActions({
    deviceId:'blind-1', blindLabel:'Bedroom Left', movementDisabled:true,
  });
  assert.match(controls, /data-action="command:open:blind-1"[^>]*disabled/);
  assert.match(controls, /data-action="command:close:blind-1"[^>]*disabled/);
  assert.match(controls, /data-action="command:stop:blind-1"[^>]*>/);
  assert.doesNotMatch(controls, /data-action="command:stop:blind-1"[^>]*disabled/);
});

test('fully open or closed blinds mute the redundant action and emphasize the next action', () => {
  const open = parseMarkup(renderBlindActions({deviceId:'blind-1',blindLabel:'Hallway',currentClosedPercent:0,currentPositionFresh:true}));
  assert.equal(open.querySelector('[data-action="command:open:blind-1"]')?.hasAttribute('disabled'), true);
  assert.match(open.querySelector('[data-action="command:close:blind-1"]')?.getAttribute('class')||'', /button--primary/);
  const closed = parseMarkup(renderBlindActions({deviceId:'blind-1',blindLabel:'Hallway',currentClosedPercent:100,currentPositionFresh:true}));
  assert.equal(closed.querySelector('[data-action="command:close:blind-1"]')?.hasAttribute('disabled'), true);
  assert.match(closed.querySelector('[data-action="command:open:blind-1"]')?.getAttribute('class')||'', /button--primary/);
  const stale = parseMarkup(renderBlindActions({deviceId:'blind-1',blindLabel:'Hallway',currentClosedPercent:100}));
  assert.equal(stale.querySelector('[data-action="command:close:blind-1"]')?.hasAttribute('disabled'), false);
});

test('percentage-position control consistently uses percent open while preserving the closed API boundary', () => {
  const controls = renderBlindActions({
    deviceId:'blind-1', blindLabel:'Hallway', supportsPosition:true,
    positionClosedPercent:47,
  });
  assert.match(controls, /type="range" min="0" max="100" step="1" value="53"/);
  assert.match(controls, /aria-label="Target position for Hallway in percent open"/);
  assert.match(controls, /data-position-output="blind-1">53%/);
  assert.match(controls, /<span class="position-end position-end--closed">Closed<\/span>/);
  assert.match(controls, /<span class="position-end position-end--open">Open<\/span>/);
  assert.match(controls, /data-position-preset="blind-1:50"/);
  assert.match(controls, /aria-label="Move Hallway to 50% open"/);
  assert.doesNotMatch(controls, /position-submit|Move to selected position/);
  assert.deepEqual(
    [...parseMarkup(controls).querySelectorAll('.position-preset .button-label')].map(label=>label.textContent),
    ['0%','25%','50%','75%','100%'],
  );
  assert.ok(controls.indexOf('data-action="command:close:blind-1"') < controls.indexOf('class="position-control"'));
  for (const preset of [0, 25, 50, 75, 100]) assert.match(controls, new RegExp(`data-position-preset="blind-1:${preset}"`));

  const disabled = renderBlindActions({
    deviceId:'blind-1', blindLabel:'Hallway', supportsPosition:true,
    positionClosedPercent:47, movementDisabled:true,
  });
  assert.match(disabled, /data-position-input="blind-1"[^>]*disabled/);
  assert.match(disabled, /data-position-preset="blind-1:50"[^>]*disabled/);
});

test('service summary names its action and renders status as text', () => {
  // Contract: UI-SERVICE-01 in docs/test-contracts.md.
  const summary = parseMarkup(renderServiceSummary('Demo <ready>'));
  const action = summary.querySelector('button');
  assert.equal(summary.querySelector('h2')?.textContent, 'Somaris service');
  assert.equal(action?.textContent, 'Open settings');
  assert.equal(action?.getAttribute('data-action'), 'settings');
  assert.equal(summary.querySelector('.subtle')?.textContent, 'Demo <ready>');
  assert.equal(summary.querySelector('.subtle img'), null);
});

test('demo banner exposes a named reset action', () => {
  const banner = parseMarkup(renderDemoBanner());
  const reset = banner.querySelector('[data-action="demo-reset"]');
  assert.equal(reset?.getAttribute('aria-label'), 'Reset demo');
  assert.equal(reset?.textContent, '');
  assert.equal(reset?.querySelector('svg')?.getAttribute('data-icon'), 'reset');
  assert.equal(reset?.getAttribute('title'), 'Reset demo');
});

test('position presets expose the selected target and its accessible action', () => {
  // Contract: UI-POSITION-01 in docs/test-contracts.md.
  const selected = parseMarkup(renderPositionPreset({
    deviceId:'blind-1', blindLabel:'Hallway', position:50, selectedPosition:50,
  })).querySelector('button');
  assert.equal(selected?.textContent, '50%');
  assert.equal(selected?.getAttribute('aria-label'), 'Move Hallway to 50% open');
  assert.equal(selected?.getAttribute('aria-pressed'), 'true');
  assert.equal(selected?.getAttribute('data-action'), 'position:blind-1:50');

  const disabled = parseMarkup(renderPositionPreset({
    deviceId:'blind-1', blindLabel:'Hallway', position:25, selectedPosition:50, disabled:true,
  })).querySelector('button');
  assert.equal(disabled?.getAttribute('aria-pressed'), 'false');
  assert.equal(disabled?.hasAttribute('disabled'), true);
});

test('direct blind graphic is a keyboard accessible percent-open slider with a drag cue', () => {
  const regular = renderBlindCard({
    id:'blind-1', label:'Hallway', availabilityLabel:'Online', availabilityStatus:'ready',
    positionPercent:42, movementStatus:'at_target', supportsPosition:true,
    showPositionCue:true,
    controlsMarkup:renderBlindActions({deviceId:'blind-1',blindLabel:'Hallway',supportsPosition:true}),
  });
  assert.match(regular, /class="blind-art[^\"]*"[^>]*role="slider"/);
  assert.match(regular, /aria-valuemin="0" aria-valuemax="100" aria-valuenow="58"/);
  assert.match(regular, /aria-label="Blind position for Hallway, percent open"/);
  assert.match(regular, /Drag the blind to set position/);

  const panel = renderBlindCard({
    id:'blind-1', label:'Hallway', availabilityLabel:'Online', availabilityStatus:'ready',
    positionPercent:42, movementStatus:'at_target', supportsPosition:true, panelMode:true,
    controlsMarkup:renderBlindActions({deviceId:'blind-1',blindLabel:'Hallway',supportsPosition:true,panelMode:true}),
  });
  assert.doesNotMatch(panel, /role="slider"/);
});

test('mobile tabs show labels below their icons and expose the current route', () => {
  const root = parseMarkup(renderMobileTabs('service'));
  const tabs = [...root.querySelectorAll('.mobile-tab')];
  assert.equal(tabs.length, 3);
  assert.deepEqual(tabs.map(tab => tab.getAttribute('aria-label')), ['Blinds', 'Activity', 'Settings']);
  assert.equal(tabs[2]?.getAttribute('aria-current'), 'page');
  assert.deepEqual(tabs.map(tab => tab.querySelector('.mobile-tab__label')?.textContent), ['Blinds', 'Activity', 'Settings']);
  assert.equal(tabs[2]?.getAttribute('href'),'#settings');
});

test('panel blind controls omit percentage positioning while other views retain it', () => {
  const panel = renderBlindActions({
    deviceId:'blind-1', blindLabel:'Hallway', panelMode:true,
    supportsPosition:true, positionClosedPercent:47,
  });
  assert.doesNotMatch(panel, /position-control|position-input|position-submit|Move to position/);
  assert.match(panel, /data-action="command:open:blind-1"/);
  assert.match(panel, /data-action="command:stop:blind-1"/);
  assert.match(panel, /data-action="command:close:blind-1"/);

  const regular = renderBlindActions({
    deviceId:'blind-1', blindLabel:'Hallway', supportsPosition:true,
    positionClosedPercent:47,
  });
  assert.match(regular, /data-position-preset="blind-1:50"/);
});

test('motor speed control uses a typed percentage and disables during calibration', () => {
  const html = renderMotorSpeedControl({deviceId:'managed-1', calibrationActive:true});
  assert.match(html, /data-motor-speed="managed-1"/);
  assert.match(html, /min="0" max="100"/);
  assert.match(html, /current setting is not read/);
  assert.match(html, /<input[^>]* disabled/);
  assert.match(html, /<button[^>]* disabled/);
});

test('endpoint calibration controls expose only Start until an explicit calibration start', () => {
  const start = renderCalibrationControls({deviceId:'managed-1'});
  assert.match(start, /command:calibration_start:managed-1/);
  assert.doesNotMatch(start, /calibration_nudge_open/);

  const active = renderCalibrationControls({deviceId:'managed-1', active:true});
  for (const action of [
    'calibration_nudge_open', 'calibration_set_open_limit',
    'calibration_nudge_closed', 'calibration_set_closed_limit',
    'stop', 'calibration_stop',
  ]) {
    assert.match(active, new RegExp(`command:${action}:managed-1`));
  }
});

test('shared button primitive escapes user text and supports icon-only accessible labels', () => {
  const markup = renderButton({
    label:'Open', accessibleLabel:'Open <Bedroom>', action:'command:open:<id>',
    icon:'open', iconOnly:true,
  });
  assert.match(markup, /aria-label="Open &lt;Bedroom&gt;"/);
  assert.match(markup, /data-action="command:open:&lt;id&gt;"/);
  assert.match(markup, /aria-hidden="true"/);
  assert.doesNotMatch(markup, /<span class="button-label">Open<\/span>/);

  const submit = parseMarkup(renderButton({
    label:'Add blind', accessibleLabel:'Add Bedroom Left', type:'submit', icon:'add', iconOnly:true,
  })).querySelector('button');
  assert.equal(submit?.getAttribute('type'), 'submit');
  assert.equal(submit?.getAttribute('aria-label'), 'Add Bedroom Left');
  assert.equal(submit?.getAttribute('data-action'), null);

  const unsafeType = 'button" onclick="alert(1)' as unknown as 'button';
  const escapedType = parseMarkup(renderButton({label:'Unsafe type', type:unsafeType})).querySelector('button');
  assert.equal(escapedType?.getAttribute('onclick'), null);
});

test('icon-only actions expose an accessible name and an actionable enabled state', () => {
  // Contract: UI-CONTROL-01 in docs/test-contracts.md.
  const root = parseMarkup(renderIconButton({label:'Refresh', action:'refresh', icon:'refresh'}));
  const button = root.querySelector('button');
  assert.ok(button);
  assert.equal(button.getAttribute('aria-label'), 'Refresh');
  assert.equal(button.getAttribute('title'), 'Refresh');
  assert.equal(button.getAttribute('data-tooltip'), 'Refresh');
  assert.equal(button.getAttribute('data-action'), 'refresh');
  assert.equal(button.textContent, '');
  assert.equal(button.querySelector('svg')?.getAttribute('aria-hidden'), 'true');

  const disabled = parseMarkup(renderIconButton({label:'Refresh', action:'refresh', icon:'refresh', disabled:true}));
  assert.notEqual(disabled.querySelector('button')?.getAttribute('disabled'), null);
});

test('icon link buttons keep a descriptive accessible name without visible label text', () => {
  const root = parseMarkup(renderIconLinkButton({
    label:'View details for Bedroom Left', action:'details:blind-1', icon:'info',
  }));
  const button = root.querySelector('button');
  assert.equal(button?.getAttribute('aria-label'), 'View details for Bedroom Left');
  assert.equal(button?.getAttribute('title'), 'View details for Bedroom Left');
  assert.equal(button?.getAttribute('data-tooltip'), 'View details for Bedroom Left');
  assert.ok(button?.classList.contains('icon-link-button'));
  assert.equal(button?.getAttribute('data-action'), 'details:blind-1');
  assert.equal(button?.querySelector('svg')?.getAttribute('data-icon'), 'info');
  assert.equal(button?.textContent, '');
});

test('confirmation panel exposes an escaped destructive choice with cancellation', () => {
  // Contract: UI-CONTROL-01 in docs/test-contracts.md.
  const markup = renderConfirmationPanel({
    title:'Remove <img src=x onerror=alert(1)>?', message:'This removes <Hallway> from Somaris.',
    confirmLabel:'Remove blind', confirmAction:'remove-confirm:blind-1',
    cancelAction:'remove-cancel:blind-1',
  });

  const root = parseMarkup(markup);
  const panel = root.querySelector('[role="group"]');
  assert.ok(panel);
  assert.equal(panel.getAttribute('aria-label'), 'Remove <img src=x onerror=alert(1)>?');
  assert.equal(panel.querySelector('h2')?.textContent, 'Remove <img src=x onerror=alert(1)>?');
  assert.equal(panel.querySelector('p')?.textContent, 'This removes <Hallway> from Somaris.');
  assert.equal(panel.querySelector('img'), null);
  const buttons = [...panel.querySelectorAll('button')];
  assert.equal(buttons.length, 2);
  assert.deepEqual(buttons.map(button => button.textContent), ['Cancel', 'Remove blind']);
  assert.deepEqual(buttons.map(button => button.getAttribute('data-action')),
    ['remove-cancel:blind-1', 'remove-confirm:blind-1']);

  const disabled = parseMarkup(renderConfirmationPanel({
    title:'Remove blind?', message:'Confirm removal.', confirmLabel:'Remove blind',
    confirmAction:'remove-confirm:blind-1', cancelAction:'remove-cancel:blind-1', disabled:true,
  }));
  assert.ok([...disabled.querySelectorAll('button')].every(button => button.hasAttribute('disabled')));
});

test('device badge and header expose escaped labels and accessible device details', () => {
  // Contract: UI-CONTROL-01 in docs/test-contracts.md.
  const badge = parseMarkup(renderBadge({label:'Stale <img src=x>', tone:'warn'}));
  assert.equal(badge.querySelector('span')?.textContent, 'Stale <img src=x>');
  assert.equal(badge.querySelector('img'), null);

  const header = parseMarkup(renderDeviceCardHeader({
    id:'blind-<1>', label:'Bedroom <Left>', availabilityLabel:'Ready',
    availabilityTone:'', availabilityStatus:'ready', isSimulated:false, lastSeen:'Seen just now',
  }));
  const details = header.querySelector('h3 button');
  assert.equal(details?.textContent, 'Bedroom <Left>');
  assert.equal(details?.getAttribute('aria-label'), 'View details for Bedroom <Left>');
  assert.equal(details?.getAttribute('data-action'), 'details:blind-<1>');
  const status = header.querySelector('[role="img"]');
  assert.equal(status?.getAttribute('aria-label'), 'Ready');
  assert.equal(header.querySelector('img'), null);
});

test('compact device headers show an icon-only freshness cue without hiding its meaning', () => {
  const panel = parseMarkup(renderDeviceCardHeader({
    id:'blind-1', label:'Hallway', availabilityLabel:'Online', availabilityStatus:'ready',
    panelMode:true, lastSeen:'Last heard from 3 min ago',
  }));
  const freshness = panel.querySelector('.freshness-indicator');
  assert.equal(freshness?.getAttribute('role'), 'img');
  assert.equal(freshness?.getAttribute('aria-label'), 'Last heard from 3 min ago');
  assert.equal(freshness?.getAttribute('title'), 'Last heard from 3 min ago');
  assert.equal(freshness?.querySelector('svg')?.getAttribute('data-icon'), 'clock');
  assert.equal(freshness?.textContent, '');
  assert.doesNotMatch(panel.textContent || '', /Last heard from 3 min ago/);

  const wide = parseMarkup(renderDeviceCardHeader({
    id:'blind-1', label:'Hallway', availabilityLabel:'Online', availabilityStatus:'ready',
    lastSeen:'Last heard from 3 min ago',
  }));
  assert.equal(wide.querySelector('.freshness-indicator'), null);
  assert.match(wide.textContent || '', /Last heard from 3 min ago/);
});

test('native disclosure primitive is icon-first and exposes a named keyboard target', () => {
  const root = parseMarkup(renderDisclosure({
    label:'Advanced device diagnostics',
    contentMarkup:'<h2>Advanced diagnostics</h2><p>Telemetry details</p>',
    variant:'advanced-disclosure',
  }));
  const details = root.querySelector('details');
  const summary = details?.querySelector('summary');
  assert.ok(details);
  assert.equal(summary?.getAttribute('aria-label'), 'Advanced device diagnostics');
  assert.equal(summary?.getAttribute('title'), 'Advanced device diagnostics');
  assert.equal(summary?.getAttribute('data-tooltip'), 'Advanced device diagnostics');
  assert.equal(summary?.querySelector('svg')?.getAttribute('data-icon'), 'chevron');
  assert.equal(summary?.textContent, '');
  assert.equal(details?.querySelector('.disclosure__content h2')?.textContent, 'Advanced diagnostics');
});

test('availability is a compact reusable icon indicator with an accessible status name', () => {
  const ready = parseMarkup(renderStatusIndicator({label:'Online', status:'ready'}));
  const indicator = ready.querySelector('[role="img"]');
  assert.equal(indicator?.getAttribute('aria-label'), 'Online');
  assert.equal(indicator?.getAttribute('title'), 'Online');
  assert.equal(indicator?.textContent, '');
  const stale = parseMarkup(renderStatusIndicator({label:'Stale', tone:'warn', status:'stale'}));
  assert.equal(stale.querySelector('[role="img"]')?.getAttribute('aria-label'), 'Stale');
});

test('compact panel notice shows a short label and retains full accessible detail', () => {
  const detail = 'Repeated Bluetooth connection failures. Open and Close are paused until the cooldown ends; Stop remains available.';
  const notice = parseMarkup(renderPanelNotice({label:'Movement paused', details:[detail], tone:'warn'}))
    .querySelector('.panel-notice');
  assert.equal(notice?.querySelector('.panel-notice__label')?.textContent, 'Movement paused');
  assert.match(notice?.getAttribute('aria-label') || '', /Movement paused/);
  assert.match(notice?.getAttribute('aria-label') || '', /Stop remains available/);
  assert.equal(notice?.getAttribute('title'), notice?.getAttribute('aria-label'));
  assert.equal(notice?.getAttribute('role'), 'status');
});

test('add-blind action shows its label and has an accessible name', () => {
  const markup = renderAddBlindButton({variant:'secondary panel-add'});
  assert.match(markup, /aria-label="Add a blind"/);
  assert.match(markup, /data-action="add"/);
  assert.match(markup, /data-icon="add"/);
  assert.match(markup, /class="button-label">Add a blind<\/span>/);
});

test('device cards expose blind identity, reported status, and named actions in both views', () => {
  // Contract: UI-CONTROL-01 in docs/test-contracts.md.
  for (const panelMode of [false, true]) {
    const card = parseMarkup(renderBlindCard({
      id:'blind-1', label:'Bedroom Left', isSimulated:false, lastSeen:'Seen just now',
      availabilityLabel:'Online', availabilityStatus:'ready', positionPercent:100,
      movementStatus:'opening',
      controlsMarkup:renderBlindActions({deviceId:'blind-1', blindLabel:'Bedroom Left', panelMode}),
      panelMode,
    }));

    assert.equal(card.querySelector('article h3 button')?.textContent, 'Bedroom Left');
    assert.equal(card.querySelector('article h3 button')?.getAttribute('aria-label'), 'View details for Bedroom Left');
    assert.equal(card.querySelector('.blind-state__label')?.textContent, 'Opening… 0%');
    assert.equal(card.querySelector('.blind-state__percent')?.textContent, '0% open');
    assert.equal(card.querySelector('.status-indicator[role="img"]')?.getAttribute('aria-label'), 'Online');
    assert.deepEqual([...card.querySelectorAll('.controls button')]
      .map(button => button.getAttribute('aria-label')),
    ['Open Bedroom Left', 'Stop Bedroom Left', 'Close Bedroom Left']);
    assert.ok([...card.querySelectorAll('.controls button')]
      .every(button => button.classList.contains('panel-target') === panelMode));
  }
});

test('panel device list omits its heading because panel count and add action live in the topbar', () => {
  const panel = renderDeviceList({
    count:3, panelMode:true, cardsMarkup:'<article>Blind</article>',
    emptyMarkup:'<div>No blinds</div>',
  });
  assert.doesNotMatch(panel, /3 blinds|<button>\+ Add<\/button>/);
  assert.doesNotMatch(panel, /<h2[^>]*>Your blinds<\/h2>/);
  assert.match(panel, /<article>Blind<\/article>/);

  const regular = renderDeviceList({count:1, panelMode:false, cardsMarkup:'<article>Blind</article>', emptyMarkup:''});
  assert.match(regular, /<h2>Your blinds<\/h2>/);
});

test('all-blinds batch controls are reusable, accessible, and disabled while unavailable or busy', () => {
  const controls = renderBatchControls({blindCount:3, eligibleCount:2});
  assert.match(controls, /aria-label="All blinds controls"/);
  assert.match(controls, /data-action="batch:open"/);
  assert.match(controls, /aria-label="Open all blinds"/);
  assert.match(controls, /data-action="batch:close"/);
  assert.match(controls, /aria-label="Close all blinds"/);
  assert.match(controls, /2 available blinds in sequence/);

  const unavailable = renderBatchControls({blindCount:3, eligibleCount:0});
  assert.match(unavailable, /data-action="batch:open"[^>]*disabled/);
  assert.match(unavailable, /data-action="batch:close"[^>]*disabled/);
  const busy = renderBatchControls({blindCount:3, eligibleCount:2, busy:true, panelMode:true});
  assert.match(busy, /all-blinds-controls--panel/);
  assert.match(busy, /data-action="batch:open"[^>]*disabled/);
  const panelControls = parseMarkup(renderBatchControls({blindCount:3, eligibleCount:2, panelMode:true}));
  assert.equal(panelControls.querySelectorAll('.all-blinds-actions .panel-target').length, 2);
  assert.deepEqual([...panelControls.querySelectorAll('.all-blinds-actions button')].map(button => button.textContent), ['', '']);
  assert.deepEqual([...panelControls.querySelectorAll('.all-blinds-actions button')].map(button => button.getAttribute('title')),
    ['Open all blinds', 'Close all blinds']);
  const regularControls = parseMarkup(renderBatchControls({blindCount:3, eligibleCount:2}));
  assert.deepEqual([...regularControls.querySelectorAll('.all-blinds-actions button')].map(button => button.textContent), ['Open all', 'Close all']);
  assert.match(renderBatchControls({blindCount:1, eligibleCount:1}), /the available blind/);

  const panelList = renderDeviceList({count:3, panelMode:true, toolbarMarkup:controls, cardsMarkup:'<article>Blind</article>'});
  const regularList = renderDeviceList({count:3, panelMode:false, toolbarMarkup:controls, cardsMarkup:'<article>Blind</article>'});
  for (const list of [panelList, regularList]) {
    assert.ok(list.indexOf('data-action="batch:open"') < list.indexOf('<article>Blind</article>'));
    assert.match(list, /data-action="batch:close"/);
  }
});

test('batch feedback distinguishes progress, acknowledgements, failures, and skipped blinds', () => {
  const running = renderBatchFeedback({
    action:'open', status:'running', target_count:3, acknowledged_count:1,
    pending_count:2, failed_count:0, skipped_count:1,
    skipped:[{label:'Back room', availability:'stale'}], targets:[],
  });
  assert.match(running, /Opening blinds/);
  assert.match(running, /1 of 3/);
  assert.match(running, /Back room/);
  assert.match(running, /stale/i);

  const unknown = renderBatchFeedback({action:'open',status:'outcome_unknown',error:'Network unavailable'});
  assert.match(unknown, /Retry original request/);
  assert.match(unknown, /Network unavailable/);
  assert.equal(renderBatchFeedback({action:'open',status:'completed',target_count:3,acknowledged_count:3}), '');
  assert.equal(renderBatchFeedback({action:'open',status:'partial',target_count:3,acknowledged_count:2,failed_count:1}), '');
  assert.equal(renderBatchFeedback({action:'open',status:'failed',target_count:3,failed_count:3}), '');
});

test('batch feedback covers each active queue stage and escapes blind details', () => {
  assert.match(renderBatchFeedback({action:'close',status:'submitting'}), /Sending close commands/);
  assert.match(renderBatchFeedback({action:'close',status:'checking'}), /Checking the close batch/);
  assert.match(renderBatchFeedback({action:'close',status:'queued',target_count:2}), /Close batch queued for 2 ready blinds/);

  const running = renderBatchFeedback({
    action:'close', status:'running', target_count:2, acknowledged_count:0,
    failed_count:1, pending_count:1,
    skipped:[{label:'<Kitchen>',availability:'stale'}],
    targets:[{label:'<Bedroom>',status:'connection_failed'}],
  });
  assert.match(running, /1 failed, 1 pending/);
  assert.match(running, /&lt;Kitchen&gt;/);
  assert.match(running, /&lt;Bedroom&gt;/);
  assert.doesNotMatch(running, /<Kitchen>|<Bedroom>/);
});

test('snackbar primitives escape content, announce by tone, and render an optional action', () => {
  const markup = renderSnackbar({
    id:'snack-1', message:'Could not add <Hallway>', tone:'error',
    action:{label:'Try again', action:'refresh'},
    details:['Skipped <Kitchen>'],
  });
  assert.match(markup, /role="alert"/);
  assert.match(markup, /aria-live="assertive"/);
  assert.match(markup, /Could not add &lt;Hallway&gt;/);
  assert.match(markup, /data-action="refresh"/);
  assert.match(markup, /aria-label="Dismiss notification"/);
  assert.match(markup, /Skipped &lt;Kitchen&gt;/);
  assert.doesNotMatch(markup, /<Hallway>/);

  const region = renderSnackbarRegion([
    {id:'snack-1',message:'Saved',tone:'success'},
    {id:'snack-2',message:'Working',tone:'info'},
  ]);
  assert.match(region, /id="snackbar-region"/);
  assert.match(region, /role="status"/);
  assert.match(region, /Saved/);
  assert.match(region, /Working/);
});

test('undo snackbar actions are icon-only and remain named',()=>{
 const root=parseMarkup(renderSnackbar({message:'Target changed',action:{label:'Undo position',action:'undo-position:blind-1:42',icon:'undo'}}));
 const action=root.querySelector('[data-snackbar-control="action"]');
 assert.equal(action?.textContent,'');
 assert.equal(action?.getAttribute('aria-label'),'Undo position');
 assert.equal(action?.getAttribute('title'),'Undo position');
 assert.equal(action?.querySelector('svg')?.getAttribute('data-icon'),'undo');
});

test('layout and theme controls are named stateful icon buttons without redundant menus',()=>{
 const root=parseMarkup(renderTopbarActions({panelMode:false,displayPreference:'panel',themePreference:'dark'}));
 const layout=root.querySelector('#layout-mode-toggle');
 const theme=root.querySelector('#theme-toggle');
 assert.equal(layout?.getAttribute('aria-label'),'Layout: Compact');
 assert.equal(layout?.getAttribute('data-action'),'layout-cycle');
 assert.equal(layout?.textContent,'');
 assert.equal(layout?.querySelector('svg')?.getAttribute('data-icon'),'panel');
 assert.equal(theme?.getAttribute('aria-label'),'Switch to light theme');
 assert.equal(theme?.textContent,'');
 assert.equal(theme?.querySelector('svg')?.getAttribute('data-icon'),'moon');
 assert.equal(root.querySelector('#layout-menu'),null);
 assert.equal(root.querySelector('#theme-menu-toggle'),null);
 assert.equal(root.querySelector('#theme-menu'),null);
 assert.equal(root.querySelector('[data-action="settings"]')?.getAttribute('aria-label'),'Settings');
 assert.equal(root.querySelector('[data-action="settings"]')?.querySelector('svg')?.getAttribute('data-icon'),'settings');
 const light=parseMarkup(renderTopbarActions({themePreference:'light'})).querySelector('#theme-toggle');
 assert.equal(light?.getAttribute('aria-label'),'Switch to dark theme');
 assert.equal(light?.querySelector('svg')?.getAttribute('data-icon'),'sun');
});

test('theme preferences remain available as a native settings control',()=>{
 const settings=parseMarkup(renderThemePreferenceControl('high-contrast'));
 const select=settings.querySelector('select[name="theme"]');
 assert.equal(settings.querySelector('h2')?.textContent,'Appearance');
 assert.equal(select?.getAttribute('aria-label'),'Theme');
 assert.equal(select?.querySelector('option:checked')?.getAttribute('value'),'high-contrast');
 assert.deepEqual([...select?.querySelectorAll('option')||[]].map(option=>option.getAttribute('value')),
  ['auto','dark','light','high-contrast']);
});

test('settings page reuses the icon-only back control and retains service diagnostics',async()=>{
 const {renderSettingsPage}=await import('../settings-page.mjs');
 const page=parseMarkup(renderSettingsPage({
  themePreference:'dark',demoMode:true,diagnostics:{demo_mode:true,simulator_ready:true},simulators:[{id:'simulator-1',advertised_name:'Demo blind'}],managedCount:1,serviceError:null,
 }));
 assert.equal(page.querySelector('h1')?.textContent,'Settings');
 const back=page.querySelector('[data-action="home"]');
 assert.equal(back?.textContent,'');
 assert.equal(back?.getAttribute('aria-label'),'Back to blinds');
 assert.equal(back?.querySelector('svg')?.getAttribute('data-icon'),'back');
 assert.equal(page.querySelector('.settings-service>h2')?.textContent,'Service status');
 assert.equal(page.querySelector('.service-health__summary')?.textContent,'Everything is working');
 assert.equal(page.querySelector('.service-health .info-item strong')?.textContent,'1');
});

test('battery and relative-time primitives show the last reading without cached jargon or duplicate info controls',()=>{
 const now=Date.parse('2026-11-19T12:00:00.000Z');
 const battery=parseMarkup(renderBatteryReading({battery:{status:'unavailable',percent_estimate:76,voltage_mv:3980,observed_at:'2026-11-19T10:00:00.000Z'},now}));
 assert.equal(battery.querySelector('[data-icon="battery"]')!==null,true);
 assert.match(battery.textContent||'',/76%/);
 assert.match(battery.textContent||'',/Last reading 2 h ago/);
 assert.equal(battery.querySelector('button'),null);
 assert.equal(battery.querySelector('[popover]'),null);
 assert.doesNotMatch(battery.textContent||'',/cached/i);
 const time=parseMarkup(renderRelativeTime('2026-11-19T11:58:00.000Z',now)).querySelector('time');
 assert.equal(time?.textContent,'2 min ago');
 assert.ok(time?.getAttribute('title'));
});

test('activity section describes the retained window, hides the empty count, and labels its utility toggle',()=>{
 const empty=parseMarkup(renderActivitySection({count:0,expanded:true,listMarkup:'<p>Changes you make will show up here.</p>'}));
 assert.equal(empty.querySelector('h2')?.textContent,'Last 30 changes');
 assert.equal(empty.querySelector('.count'),null);
 assert.equal(empty.querySelector('p')?.textContent,'Changes you make will show up here.');
 assert.equal(empty.querySelector('[data-action="activity-toggle"]')?.textContent?.trim(),'Hide');
 const filled=parseMarkup(renderActivitySection({count:4,expanded:false,listMarkup:'<ol></ol>'}));
 assert.equal(filled.querySelector('.count')?.textContent,'4 changes');
 assert.equal(filled.querySelector('[data-action="activity-toggle"]')?.textContent?.trim(),'Show');
});

test('service health summary has a plain-language lead and demo-only details can be grouped',()=>{
 const page=parseMarkup(renderServiceHealthSummary({summary:'Everything is working',fields:[{label:'Simulated blinds',value:4}],notUsedMarkup:renderInfoGrid([{label:'Bluetooth adapter',value:'Not used in demo'}])}));
 assert.equal(page.querySelector('.service-health__summary')?.textContent,'Everything is working');
 assert.equal(page.querySelector('.service-health__details .info-item strong')?.textContent,'4');
 assert.equal(page.querySelector('details summary')?.textContent,'Not used in demo');
});

test('dark muted and footer token colors exceed AA text contrast',()=>{
 const css=readFileSync(new URL('../style.css',import.meta.url),'utf8');
 const token=(name:string)=>css.match(new RegExp(`${name}:(#[0-9a-fA-F]{6})`))?.[1]||'';
 const luminance=(hex:string)=>{
  const channels=[1,3,5].map(index=>parseInt(hex.slice(index,index+2),16)/255).map(value=>value<=.04045?value/12.92:((value+.055)/1.055)**2.4);
  return channels[0]!*0.2126+channels[1]!*0.7152+channels[2]!*0.0722;
 };
 const ratio=(foreground:string,background:string)=>{const [high,low]=[luminance(foreground),luminance(background)].sort((a,b)=>b-a);return(high+.05)/(low+.05);};
 const background=token('--token-bg');
 assert.ok(ratio(token('--token-muted'),background)>=4.5);
 assert.ok(ratio(token('--token-footer'),background)>=4.5);
});

test('shared control tokens replace invalid slider CSS and panel size patches',()=>{
 const style=readFileSync(new URL('../style.css',import.meta.url),'utf8');
 const components=readFileSync(new URL('../components.css',import.meta.url),'utf8');
 const panel=readFileSync(new URL('../panel.css',import.meta.url),'utf8');
 assert.match(style,/--control-target-size:44px/);
 assert.match(style,/--status-indicator-size:32px/);
 assert.doesNotMatch(style,/aria-valuenow\s*:/);
 assert.match(components,/\.icon-button,\.topbar-icon-button\{width:var\(--control-target-size\)/);
 assert.doesNotMatch(panel,/\.icon-button\s*\{\s*(?:width|height):|\.topbar-add\s*\{[^}]*\b(?:width|height):|\.status-indicator\s*\{[^}]*\b(?:width|height):/);
 assert.match(components,/\.snackbar--info/);
});

test('topbar reuses named icon controls for settings, layout, theme, reset, and refresh', () => {
  const panel = renderTopbarActions({blindCount:3, panelMode:true, demoMode:true, displayPreference:'panel'});
  assert.match(panel, /class="topbar-device-count">3 blinds<\/span>/);
  assert.match(panel, /data-action="add"[^>]*aria-label="Add a blind"/);
  assert.match(panel, /data-icon="add"/);
  assert.match(panel, /id="layout-mode-toggle"/);
  assert.match(panel, /data-action="layout-cycle"/);
  assert.match(panel, /id="theme-toggle"/);
  assert.doesNotMatch(panel, /id="layout-menu"|id="theme-menu"|More themes/);
  assert.match(panel, /data-action="refresh"[^>]*aria-label="Refresh"/);
  assert.match(panel, /data-icon="refresh"/);
  assert.match(panel, />Add a blind<\/span>/);
  assert.match(panel, />Refresh<\/span>/);
  assert.match(panel, /data-action="demo-reset"[^>]*aria-label="Reset demo"/);
  const regular=renderTopbarActions({panelMode:false});
  assert.match(regular, /data-action="settings"[^>]*aria-label="Settings"/);

  for (const displayPreference of ['mobile', 'desktop']) {
    const regular = renderTopbarActions({blindCount:3, panelMode:false, displayPreference});
    assert.doesNotMatch(regular, /topbar-device-count/);
    assert.match(regular, /data-action="add"/);
    assert.match(regular, /data-icon="add"/);
    assert.match(regular, />Add a blind<\/span>/);
  }
});

test('panel topbar keeps an accessible icon-only demo reset button when the banner is compacted away', () => {
  const panel = renderTopbarActions({panelMode:true, demoMode:true});
  const reset = parseMarkup(panel).querySelector('[data-action="demo-reset"]');
  assert.ok(reset);
  assert.equal(reset.getAttribute('aria-label'),'Reset demo');
  assert.equal(reset.textContent,'');
  assert.equal(reset.querySelector('svg')?.getAttribute('data-icon'),'reset');
  assert.equal(reset.getAttribute('title'),'Reset demo');
});

test('room-aware cards show their room label and preserve a shared collection grid', () => {
  // Contract: UI-ROOM-01 in docs/test-contracts.md.
  const markup = renderBlindCard({
    id:'demo-living', label:'Living Room — Main Window', room:'Living Room', isSimulated:true,
    lastSeen:'', availabilityLabel:'Online', positionPercent:75, showRoom:true,
    movementStatus:'at_target', controlsMarkup:renderBlindActions({
      deviceId:'demo-living', blindLabel:'Living Room — Main Window',
    }),
  });
  const card = parseMarkup(markup);
  assert.equal(card.querySelector('.card-title')?.textContent, 'Main Window');
  assert.equal(card.querySelector('.room-label')?.textContent, 'Living Room');
  assert.equal(card.querySelector('.device-kind'), null);
  assert.equal(card.querySelector('.last-seen'), null);
  assert.ok(card.querySelector('.card-title-chevron'));
  assert.equal(card.querySelector('.blind-state__label')?.textContent, 'Partially open');
  const caseVariant = parseMarkup(renderBlindCard({
    id:'demo-living-side', label:'living room - Side Window', room:'Living Room', isSimulated:true,
    lastSeen:'', availabilityLabel:'Online', positionPercent:50,
    movementStatus:'at_target', controlsMarkup:'',
  }));
  assert.equal(caseVariant.querySelector('.card-title')?.textContent, 'Side Window');
  const groupsMarkup=renderRoomGroup({room:'Bedroom'})+renderRoomGroup({room:'Living Room'});
  const collection=parseMarkup(renderRoomCollection({groupsMarkup,cardsMarkup:markup+renderBlindCard({
    id:'bedroom',label:'Blackout Blind',room:'Bedroom',showRoom:true,availabilityLabel:'Online',positionPercent:0,
    movementStatus:'at_target',controlsMarkup:'',
  })}));
  assert.equal(collection.querySelectorAll('.room-collection__grid').length,1);
  assert.equal(collection.querySelector('.room-collection__grid')?.querySelectorAll('.card').length,2);
  assert.deepEqual([...collection.querySelectorAll('.room-heading')].map(heading=>heading.textContent),['Bedroom','Living Room']);
  assert.deepEqual(
    [...card.querySelectorAll('.controls button')].map(button => [
      button.getAttribute('aria-label'), button.getAttribute('data-action'),
    ]),
    [
      ['Open Living Room — Main Window', 'command:open:demo-living'],
      ['Stop Living Room — Main Window', 'command:stop:demo-living'],
      ['Close Living Room — Main Window', 'command:close:demo-living'],
    ],
  );
});

test('room picker offers Unassigned, existing names, and a custom room field', () => {
  const picker = parseMarkup(renderRoomPicker({
    id:'blind-room',
    rooms:['Living Room','Study'],
    selectedRoom:'Study',
  }));
  const select = picker.querySelector<HTMLSelectElement>('[data-room-choice]');
  assert.ok(select);
  assert.deepEqual([...select.options].map(option => option.textContent), [
    'Unassigned','Living Room','Study','Create a room…',
  ]);
  assert.equal(select.options[2]?.getAttribute('selected'), '');
  assert.equal(select.getAttribute('aria-label'), 'Room');
  assert.equal(select.getAttribute('aria-describedby'), 'blind-room-error');
  const custom = picker.querySelector<HTMLInputElement>('[name="room_custom"]');
  assert.equal(custom?.getAttribute('maxlength'), '40');
  assert.equal(custom?.getAttribute('aria-describedby'), 'blind-room-custom-error');
  assert.equal(custom?.closest('[data-room-custom-field]')?.hasAttribute('hidden'), true);
});

test('room group provides reusable, accessible management actions', () => {
  const group = parseMarkup(renderRoomGroup({
    room:'Unassigned',
    actionsMarkup:'<button type="button" data-action="room-rename">Rename room</button>',
  }));
  assert.equal(group.querySelector('.room-heading')?.textContent, 'Unassigned');
  assert.equal(group.querySelector('[data-action="room-rename"]')?.textContent, 'Rename room');
  assert.match(group.querySelector('[data-room-group]')?.getAttribute('data-room-group') || '', /Unassigned/);
});

test('panel home omits supporting sections while regular view retains activity and service', () => {
  assert.equal(renderHomeSupportingSections({panelMode:true, activityMarkup:'ACTIVITY', serviceMarkup:'SERVICE'}), '');
  assert.equal(
    renderHomeSupportingSections({panelMode:false, activityMarkup:'ACTIVITY', serviceMarkup:'SERVICE'}),
    'ACTIVITYSERVICE',
  );
});

test('info grid component supports both escaped values and reusable position markup', () => {
  const markup = renderInfoGrid([
    {label:'Availability', value:'Ready'},
    {label:'Position', contentMarkup:renderBlindStatus({positionPercent:25, movementStatus:'at_target', compact:true})},
  ]);
  assert.match(markup, /Availability/);
  assert.match(markup, /Ready/);
  assert.match(markup, /Partially open/);
  assert.match(markup, /75% open/);
  assert.match(renderInfoGrid([{label:'Name', value:'<Hallway>'}]), /&lt;Hallway&gt;/);
});
