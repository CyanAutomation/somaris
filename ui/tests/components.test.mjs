import {test} from 'node:test';
import assert from 'node:assert/strict';
import {
  blindState,
  movementStatusFromPositions,
  renderBlindActions,
  renderBlindCard,
  renderCalibrationControls,
  renderBlindStatus,
  renderMotorSpeedControl,
  renderBadge,
  renderStatusIndicator,
  renderDeviceCardHeader,
  renderButton,
  renderIconButton,
  renderAddBlindButton,
  renderDestructiveButton,
  renderConfirmationPanel,
  renderDeviceList,
  renderBatchControls,
  renderBatchFeedback,
  renderSnackbar,
  renderSnackbarRegion,
  renderTopbarActions,
  renderHomeSupportingSections,
  renderInfoGrid,
} from '../components.mjs';

test('blind state prioritizes reported movement and otherwise describes current position', () => {
  assert.deepEqual(blindState(100, 'opening'), {label:'Opening', percentClosed:100});
  assert.deepEqual(blindState(0, 'closing'), {label:'Closing', percentClosed:0});
  assert.deepEqual(blindState(0, 'at_target'), {label:'Open', percentClosed:0});
  assert.deepEqual(blindState(100, 'at_target'), {label:'Closed', percentClosed:100});
  assert.deepEqual(blindState(42, 'at_target'), {label:'Partially open', percentClosed:42});
  assert.deepEqual(blindState(null, 'unavailable'), {label:'Position unavailable', percentClosed:null});
  assert.deepEqual(blindState(101, 'at_target'), {label:'Position unavailable', percentClosed:null});
});

test('movement direction follows the current and target closed percentages', () => {
  assert.equal(movementStatusFromPositions(20, 80), 'closing');
  assert.equal(movementStatusFromPositions(80, 20), 'opening');
  assert.equal(movementStatusFromPositions(40, 40), 'at_target');
  assert.equal(movementStatusFromPositions(null, 40), 'unavailable');
});

test('blind status renders state before secondary percentage', () => {
  const markup = renderBlindStatus({positionPercent:42, movementStatus:'at_target'});
  assert.match(markup, /blind-state__label">Partially open/);
  assert.match(markup, /blind-state__percent">42% closed/);
  assert.ok(markup.indexOf('Partially open') < markup.indexOf('42% closed'));
  assert.doesNotMatch(renderBlindStatus({positionPercent:null, movementStatus:'unavailable'}), /% closed/);
});

test('icon-only blind actions have named controls, visible glyphs and disabled states', () => {
  const actions = renderBlindActions({deviceId:'blind-1', blindLabel:'Bedroom Left', panelMode:true, disabled:false});
  assert.match(actions, /role="group" aria-label="Bedroom Left controls"/);
  assert.match(actions, /aria-label="Open Bedroom Left"/);
  assert.match(actions, /title="Open Bedroom Left"/);
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

test('percentage-position control reflects the observed target and has an accessible label', () => {
  const controls = renderBlindActions({
    deviceId:'blind-1', blindLabel:'Hallway', supportsPosition:true,
    positionClosedPercent:47,
  });
  assert.match(controls, /type="range" min="0" max="100" step="1" value="47"/);
  assert.match(controls, /aria-label="Target position for Hallway in percent closed"/);
  assert.match(controls, /data-position-output="blind-1">47%/);
  assert.match(controls, /data-action="position:blind-1"/);

  const disabled = renderBlindActions({
    deviceId:'blind-1', blindLabel:'Hallway', supportsPosition:true,
    positionClosedPercent:47, movementDisabled:true,
  });
  assert.match(disabled, /data-position-input="blind-1"[^>]*disabled/);
  assert.match(disabled, /data-action="position:blind-1"[^>]*disabled/);
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
  assert.match(regular, /data-action="position:blind-1"/);
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
});

test('reusable icon button primitive provides consistent accessible icon controls', () => {
  const markup = renderIconButton({label:'Refresh', action:'refresh', icon:'refresh'});
  assert.match(markup, /class="button icon-button icon-action"/);
  assert.match(markup, /aria-label="Refresh"/);
  assert.match(markup, /title="Refresh"/);
  assert.match(markup, /data-action="refresh"/);
  assert.match(markup, /data-icon="refresh"/);
  assert.doesNotMatch(markup, /button-label/);
});

test('destructive action primitive provides a visible danger button with an icon', () => {
  const markup = renderDestructiveButton({label:'Remove blind', action:'remove:blind-1'});
  assert.match(markup, /class="button danger"/);
  assert.match(markup, /data-action="remove:blind-1"/);
  assert.match(markup, /data-icon="trash"/);
  assert.match(markup, /<span class="button-label">Remove blind<\/span>/);
  assert.match(renderDestructiveButton({label:'Remove <blind>', action:'remove:x', disabled:true}), /disabled/);
});

test('reusable confirmation panel escapes copy and composes cancel and destructive actions', () => {
  const markup = renderConfirmationPanel({
    title:'Remove <Hallway>?', message:'This removes <Hallway> from Somaris.',
    confirmLabel:'Remove blind', confirmAction:'remove-confirm:blind-1',
    cancelAction:'remove-cancel:blind-1',
  });
  assert.match(markup, /role="group" aria-label="Remove &lt;Hallway&gt;\?"/);
  assert.match(markup, /This removes &lt;Hallway&gt; from Somaris\./);
  assert.match(markup, /data-action="remove-cancel:blind-1"/);
  assert.match(markup, /data-action="remove-confirm:blind-1"/);
  assert.match(markup, /class="button danger"/);
  const disabled = renderConfirmationPanel({
    title:'Remove blind?', message:'Confirm removal.', confirmLabel:'Remove blind',
    confirmAction:'remove-confirm:blind-1', cancelAction:'remove-cancel:blind-1', disabled:true,
  });
  assert.equal((disabled.match(/ disabled/g) || []).length, 2);
});

test('shared badge and device-card header primitives keep card markup consistent and escaped', () => {
  assert.equal(renderBadge({label:'Ready', tone:''}), '<span class="badge">Ready</span>');
  assert.equal(renderBadge({label:'Stale <blind>', tone:'warn'}), '<span class="badge warn">Stale &lt;blind&gt;</span>');

  const header = renderDeviceCardHeader({
    id:'blind-<1>', label:'Bedroom <Left>', availabilityLabel:'Ready',
    availabilityTone:'', availabilityStatus:'ready', isSimulated:false, lastSeen:'Seen just now',
  });
  assert.match(header, /class="card-top device-card-header"/);
  assert.match(header, /data-action="details:blind-&lt;1&gt;"/);
  assert.match(header, /aria-label="View details for Bedroom &lt;Left&gt;"/);
  assert.match(header, /class="status-indicator" role="img" aria-label="Ready" title="Ready"/);
});

test('availability is a compact reusable icon indicator with an accessible status name', () => {
  const ready = renderStatusIndicator({label:'Observed recently', status:'ready'});
  assert.match(ready, /class="status-indicator"/);
  assert.match(ready, /aria-label="Observed recently"/);
  assert.match(ready, /title="Observed recently"/);
  assert.match(ready, /data-icon="observed"/);
  assert.doesNotMatch(ready, />Observed recently</);
  assert.match(renderStatusIndicator({label:'Stale', tone:'warn', status:'stale'}), /data-icon="stale"/);
});

test('add-blind action is a plus-only button with an accessible name and tooltip', () => {
  const markup = renderAddBlindButton({variant:'secondary panel-add'});
  assert.match(markup, /aria-label="Add a blind"/);
  assert.match(markup, /title="Add a blind"/);
  assert.match(markup, /data-action="add"/);
  assert.match(markup, /data-icon="add"/);
  assert.doesNotMatch(markup, /class="button-label"/);
});

test('device card composes the state, named actions and reusable content areas', () => {
  const card = renderBlindCard({
    id:'blind-1', label:'Bedroom Left', isSimulated:false, lastSeen:'Seen just now',
    availabilityLabel:'Ready', availabilityTone:'', availabilityStatus:'ready', positionPercent:100,
    movementStatus:'opening', blindLines:11,
    controlsMarkup:renderBlindActions({deviceId:'blind-1', blindLabel:'Bedroom Left', panelMode:true, disabled:false}),
    messageMarkup:'', footerMarkup:'', panelMode:true,
  });
  assert.match(card, /class="card card--panel"/);
  assert.match(card, /blind-state__label">Opening/);
  assert.match(card, /blind-state__percent">100% closed/);
  assert.match(card, /aria-label="Open Bedroom Left"/);
  assert.match(card, /Bedroom Left/);
  assert.match(card, /blind-state--panel/);
});

test('regular device cards keep the shared components without panel-only variants', () => {
  const card = renderBlindCard({
    id:'blind-2', label:'Study', availabilityLabel:'Ready', availabilityStatus:'ready', positionPercent:25,
    movementStatus:'at_target', controlsMarkup:'<div class="controls"></div>',
  });
  assert.match(card, /<article class="card">/);
  assert.doesNotMatch(card, /card--panel|blind-state--panel/);
  assert.match(card, /class="status-indicator" role="img" aria-label="Ready"/);
});

test('panel device list omits its heading because panel count and add action live in the topbar', () => {
  const panel = renderDeviceList({
    count:3, panelMode:true, cardsMarkup:'<article>Blind</article>',
    emptyMarkup:'<div>No blinds</div>',
  });
  assert.doesNotMatch(panel, /3 blinds|<button>\+ Add<\/button>/);
  assert.doesNotMatch(panel, /<h2[^>]*>Your blinds<\/h2>/);
  assert.match(panel, /<article>Blind<\/article>/);

  const regular = renderDeviceList({count:1, panelMode:false, addButton:'', cardsMarkup:'<article>Blind</article>', emptyMarkup:''});
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

test('topbar composes the plus-only add action beside a named mode toggle and refresh controls', () => {
  const panel = renderTopbarActions({blindCount:3, panelMode:true, displayPreference:'panel'});
  assert.match(panel, /class="topbar-device-count">3 blinds<\/span>/);
  assert.match(panel, /data-action="add"[^>]*aria-label="Add a blind"/);
  assert.match(panel, /data-icon="add"/);
  assert.match(panel, /id="display-mode-toggle"/);
  assert.match(panel, /aria-label="Display mode: Panel\. Switch to Desktop"/);
  assert.match(panel, /data-action="display-mode"/);
  assert.match(panel, /data-icon="display"/);
  assert.doesNotMatch(panel, /<select|<details class="display-menu"/);
  assert.match(panel, /data-action="refresh"[^>]*aria-label="Refresh"/);
  assert.match(panel, /data-icon="refresh"/);
  assert.ok(panel.indexOf('data-action="add"') < panel.indexOf('data-action="display-mode"'));
  assert.ok(panel.indexOf('data-action="display-mode"') < panel.indexOf('data-action="refresh"'));
  assert.doesNotMatch(panel, />Add a blind</);

  for (const displayPreference of ['mobile', 'desktop']) {
    const regular = renderTopbarActions({blindCount:3, panelMode:false, displayPreference});
    assert.doesNotMatch(regular, /topbar-device-count/);
    assert.match(regular, /data-action="add"/);
    assert.match(regular, /data-icon="add"/);
    assert.doesNotMatch(regular, />Add a blind</);
  }
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
  assert.match(markup, /25% closed/);
  assert.match(renderInfoGrid([{label:'Name', value:'<Hallway>'}]), /&lt;Hallway&gt;/);
});
