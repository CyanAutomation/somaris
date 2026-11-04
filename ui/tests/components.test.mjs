import {test} from 'node:test';
import assert from 'node:assert/strict';
import {
  blindState,
  movementStatusFromPositions,
  renderBlindActions,
  renderBlindCard,
  renderBlindStatus,
  renderBadge,
  renderDeviceCardHeader,
  renderButton,
  renderAddBlindButton,
  renderDeviceList,
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

test('shared badge and device-card header primitives keep card markup consistent and escaped', () => {
  assert.equal(renderBadge({label:'Ready', tone:''}), '<span class="badge">Ready</span>');
  assert.equal(renderBadge({label:'Stale <blind>', tone:'warn'}), '<span class="badge warn">Stale &lt;blind&gt;</span>');

  const header = renderDeviceCardHeader({
    id:'blind-<1>', label:'Bedroom <Left>', availabilityLabel:'Ready',
    availabilityTone:'', isSimulated:false, lastSeen:'Seen just now',
  });
  assert.match(header, /class="card-top device-card-header"/);
  assert.match(header, /data-action="details:blind-&lt;1&gt;"/);
  assert.match(header, /aria-label="View details for Bedroom &lt;Left&gt;"/);
  assert.match(header, /class="badge">Ready<\/span>/);
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
    availabilityLabel:'Ready', availabilityTone:'', positionPercent:100,
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
    id:'blind-2', label:'Study', availabilityLabel:'Ready', positionPercent:25,
    movementStatus:'at_target', controlsMarkup:'<div class="controls"></div>',
  });
  assert.match(card, /<article class="card">/);
  assert.doesNotMatch(card, /card--panel|blind-state--panel/);
  assert.match(card, /class="badge">Ready<\/span>/);
});

test('panel device list keeps count and add action without a visible section title', () => {
  const panel = renderDeviceList({
    count:3, panelMode:true, addButton:'<button>+ Add</button>', cardsMarkup:'<article>Blind</article>',
    emptyMarkup:'<div>No blinds</div>',
  });
  assert.match(panel, /3 blinds/);
  assert.match(panel, /<button>\+ Add<\/button>/);
  assert.doesNotMatch(panel, /<h2[^>]*>Your blinds<\/h2>/);
  assert.match(panel, /<article>Blind<\/article>/);

  const regular = renderDeviceList({count:1, panelMode:false, addButton:'', cardsMarkup:'<article>Blind</article>', emptyMarkup:''});
  assert.match(regular, /<h2>Your blinds<\/h2>/);
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
