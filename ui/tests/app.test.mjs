import {test} from 'node:test';
import assert from 'node:assert/strict';
import {activityMessage, availability, commandMessage, displayPreferenceFrom, escapeHtml, gattReadinessLabel, observationFor, resolveDisplayMode} from '../app.mjs';

test('automatic display mode distinguishes square panels from phones and desktop windows',()=>{
 assert.equal(resolveDisplayMode('auto', 800, 800), 'panel');
 assert.equal(resolveDisplayMode('auto', 390, 390), 'panel');
 assert.equal(resolveDisplayMode('auto', 390, 844), 'mobile');
 assert.equal(resolveDisplayMode('auto', 1440, 900), 'desktop');
 assert.equal(resolveDisplayMode('auto', 800, 560), 'desktop');
});

test('display preference query pins a mode and otherwise uses the saved choice',()=>{
 assert.equal(displayPreferenceFrom('?view=panel', 'desktop'), 'panel');
 assert.equal(displayPreferenceFrom('?view=unknown', 'mobile'), 'mobile');
 assert.equal(displayPreferenceFrom('', 'panel'), 'panel');
 assert.equal(displayPreferenceFrom('', 'unknown'), 'auto');
});

test('explicit display mode overrides automatic viewport selection',()=>{
 assert.equal(resolveDisplayMode('panel', 390, 844), 'panel');
 assert.equal(resolveDisplayMode('mobile', 1440, 900), 'mobile');
 assert.equal(resolveDisplayMode('desktop', 390, 844), 'desktop');
 assert.equal(resolveDisplayMode('unknown', 390, 844), 'mobile');
});

test('command messages distinguish acknowledgement and uncertain outcomes',()=>{
 assert.match(commandMessage({status:'write_acknowledged'})[0],/acknowledged/i);
 assert.match(commandMessage({status:'outcome_unknown'})[0],/check.*position/i);
 assert.match(commandMessage({status:'write_failed'})[0],/uncertain/i);
 assert.match(commandMessage({status:'connection_failed',retryable:true})[0],/no command was sent/i);
});
test('fresh observations are not presented as GATT command readiness',()=>{
 assert.equal(availability('ready')[0], 'Observed recently');
 assert.equal(gattReadinessLabel({status:'untested'}), 'No command sent yet');
 assert.equal(gattReadinessLabel({status:'connection_failed'}), 'Could not connect; no write sent');
 assert.equal(gattReadinessLabel({status:'write_acknowledged'}), 'GATT write acknowledged');
 assert.equal(gattReadinessLabel({status:'outcome_unknown'}), 'Write outcome unknown');
});
test('observations are associated only with a unique matching identity and source',()=>{
 const d={enrollment:{logical_id:'x',advertised_name:'Blind',is_simulated:false}};
 const o={logical_id:'x',advertised_name:'Blind',is_simulated:false};
 assert.equal(observationFor(d,[o]),o);
 assert.equal(observationFor(d,[o,o]),null);
 assert.equal(observationFor(d,[{...o,is_simulated:true}]),null);
});
test('device-provided text is escaped',()=>assert.equal(escapeHtml('<img src="x">'), '&lt;img src=&quot;x&quot;&gt;'));

test('service refresh loads the simulator collection', async () => {
 const {refreshService} = await import('../app.mjs');
 const simulators = [{id:'simulator-1', advertised_name:'Somaris Simulator 1'}, {id:'simulator-2', advertised_name:'Somaris Simulator 2'}];
 const calls = [];
 const request = async path => {
  calls.push(path);
  if (path === '/api/dev/simulators') return simulators;
  if (path === '/api/v1/diagnostics') return {scanner:{adapter_available:true}};
  return [];
 };
 const state = {};
 await refreshService(state, request);
 assert.ok(calls.includes('/api/dev/simulators'));
 assert.deepEqual(state.simulators, simulators);
 assert.equal(state.serviceError, null);
});

test('service refresh loads recent activity and activity descriptions stay readable', async () => {
 const {refreshService} = await import('../app.mjs');
 const activity = [{kind:'command', action:'close', subject:'Study blind', status:'write_acknowledged'}];
 const calls = [];
 const request = async path => {
  calls.push(path);
  if (path === '/api/control/v1/audit-log') return activity;
  if (path === '/api/v1/diagnostics') return {scanner:{adapter_available:true}};
  return [];
 };
 const state = {};
 await refreshService(state, request);
 assert.ok(calls.includes('/api/control/v1/audit-log'));
 assert.deepEqual(state.activity, activity);
 assert.equal(activityMessage(activity[0]), 'Close command · write acknowledged · Study blind');
 assert.equal(activityMessage({kind:'enrollment_removed', subject:'Study blind'}), 'Removed Study blind');
});

test('enrollment errors survive successful and failed service refreshes until retry or dismissal', async () => {
 const {enrollBlind, refreshService, dismissEnrollmentError} = await import('../app.mjs');
 const state = {enrollmentError:null, serviceError:null};
 let failRefresh = false;
 const request = async (path, options) => {
  if (options?.method === 'POST') throw new Error('Device is no longer fresh');
  if (failRefresh) throw new Error('Network unavailable');
  return path.endsWith('/diagnostics') ? {scanner:{adapter_available:true}} : [];
 };
 const refresh = () => refreshService(state, request);
 assert.equal(await enrollBlind(state, request, refresh, 'blind-1', 'Hallway'), false);
 assert.match(state.enrollmentError, /Device is no longer fresh/);
 assert.equal(state.serviceError, null);
 await refresh();
 assert.match(state.enrollmentError, /Device is no longer fresh/);
 failRefresh = true;
 await refresh();
 assert.match(state.serviceError, /Network unavailable/);
 assert.match(state.enrollmentError, /Device is no longer fresh/);
 dismissEnrollmentError(state);
 assert.equal(state.enrollmentError, null);
 assert.match(state.serviceError, /Network unavailable/);
 failRefresh = false;
 const successfulRequest = async (path, options) => options?.method === 'POST' ? {} : request(path, options);
 state.enrollmentError = 'previous failure';
 assert.equal(await enrollBlind(state, successfulRequest, refresh, 'blind-1', 'Hallway'), true);
 assert.equal(state.enrollmentError, null);
 assert.equal(state.serviceError, null);
});

test('battery display shows estimated percentage and retains cached and unavailable states', async () => {
 const {batteryDisplay} = await import('../app.mjs');
 const observed = batteryDisplay({status:'observed', percent_estimate:76, voltage_mv:3980});
 assert.match(observed, /Battery ~76%/);
 assert.match(observed, /Battery voltage: 3980 mV/);
 assert.match(observed, /<details class="battery-info">/);
 assert.doesNotMatch(observed, /cached/);
 assert.match(batteryDisplay({status:'unavailable', percent_estimate:0, voltage_mv:3600}), /Battery ~0% · cached/);
 assert.match(batteryDisplay({status:'observed', percent_estimate:100}), /Battery ~100%/);
 assert.match(batteryDisplay({status:'observed', voltage_mv:4120}), /Battery unavailable/);
 assert.match(batteryDisplay(null), /Battery unavailable/);
});

test('removing an enrollment sends DELETE, clears local device state, and refreshes', async () => {
 const {removeEnrolledBlind} = await import('../app.mjs');
 const id = 'managed/a b';
 const token = Symbol('command');
 const state = {
  managed:[{enrollment:{id}},{enrollment:{id:'other'}}],
  commands:{[id]:{status:'write_acknowledged'}},
  busy:new Set([id]), tokens:{[id]:token},
  confirmingRemovalId:id, removalError:'old error',
 };
 const calls = [];
 let refreshes = 0;
 const request = async (...args) => { calls.push(args); };
 const refresh = async () => { refreshes += 1; };

 assert.equal(await removeEnrolledBlind(state, request, refresh, id), true);
 assert.deepEqual(calls, [[`/api/control/v1/devices/${encodeURIComponent(id)}`, {method:'DELETE'}]]);
 assert.deepEqual(state.managed, [{enrollment:{id:'other'}}]);
 assert.equal(state.commands[id], undefined);
 assert.equal(state.busy.has(id), false);
 assert.equal(state.tokens[id], undefined);
 assert.equal(state.confirmingRemovalId, null);
 assert.equal(state.removingEnrollmentId, null);
 assert.equal(state.removalError, null);
 assert.equal(refreshes, 1);
});

test('removal failures remain visible and keep the confirmation available for retry', async () => {
 const {removeEnrolledBlind} = await import('../app.mjs');
 const id = 'managed-1';
 const state = {managed:[{enrollment:{id}}], confirmingRemovalId:id, removalError:null};
 let refreshes = 0;
 const request = async () => { throw new Error('The enrollment has queued or running commands.'); };

 assert.equal(await removeEnrolledBlind(state, request, async () => { refreshes += 1; }, id), false);
 assert.match(state.removalError, /queued or running commands/);
 assert.match(state.removalError, /refresh/i);
 assert.equal(state.managed.length, 1);
 assert.equal(state.confirmingRemovalId, id);
 assert.equal(state.removingEnrollmentId, null);
 assert.equal(refreshes, 1);
});

test('new removal confirmations take focus before surviving actions, then preserve or restore focus', async () => {
 const {resolveActionFocus} = await import('../app.mjs');
 const actions = ['dismiss-removal', 'remove:blind-1', 'remove-cancel:blind-1', 'remove-confirm:blind-1'];

 assert.equal(resolveActionFocus({
  focusedAction:'dismiss-removal', pendingConfirmationId:'blind-1',
  confirmingRemovalId:'blind-1', availableActions:actions,
 }), 'remove-cancel:blind-1');
 assert.equal(resolveActionFocus({
  focusedAction:'remove-confirm:blind-1', confirmingRemovalId:'blind-1', availableActions:actions,
 }), 'remove-confirm:blind-1');
 assert.equal(resolveActionFocus({
  focusedAction:'remove-cancel:blind-1', availableActions:['remove:blind-1'],
 }), 'remove:blind-1');
});
