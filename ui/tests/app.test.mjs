import {test} from 'node:test';
import assert from 'node:assert/strict';
import {activityMessage, availability, batchEligibleCount, commandMessage, displayPreferenceFrom, escapeHtml, gattReadinessLabel, gattTransportEvidenceLabel, handleCommandPollFailure, motionFeedbackLabel, observationFor, refreshCommandTelemetry, refreshService, resolveDisplayMode} from '../app.mjs';

test('automatic display mode distinguishes square panels from phones and desktop windows',()=>{
 assert.equal(resolveDisplayMode('auto', 800, 800), 'panel');
 assert.equal(resolveDisplayMode('auto', 390, 390), 'panel');
 assert.equal(resolveDisplayMode('auto', 390, 844), 'mobile');
 assert.equal(resolveDisplayMode('auto', 1440, 900), 'desktop');
 assert.equal(resolveDisplayMode('auto', 800, 560), 'desktop');
});

test('all-blinds batch submission is idempotent and a failed request stays locked for safe recovery', async () => {
 const {submitBlindBatch, checkBlindBatch} = await import('../app.mjs');
 const state = {batch:null};
 const requests = [];
 const request = async (path, options) => {
  requests.push([path, options]);
  const action = options?.body ? JSON.parse(options.body).action : 'open';
  return {id:'batch-1', action, status:'queued', target_count:2, acknowledged_count:0, failed_count:0, pending_count:2, skipped_count:0, skipped:[], targets:[]};
 };
 let changes = 0;
 assert.equal(await submitBlindBatch(state, request, 'open', () => { changes += 1; }, () => 'key-1'), true);
 assert.equal(requests.length, 1);
 assert.equal(requests[0][0], '/api/control/v1/batches');
 assert.equal(requests[0][1].headers['Idempotency-Key'], 'key-1');
 assert.deepEqual(JSON.parse(requests[0][1].body), {action:'open'});
 assert.equal(state.batch.id, 'batch-1');
 assert.equal(state.batch.status, 'queued');
 assert.equal(changes, 2);
 assert.equal(await submitBlindBatch(state, request, 'close', () => {}, () => 'key-2'), false);
 assert.equal(requests.length, 1);

 const unknownState = {batch:null};
 const failedRequest = async (path, options) => {
  requests.push([path, options]);
  throw new Error('Network unavailable');
 };
 assert.equal(await submitBlindBatch(unknownState, failedRequest, 'close', () => {}, () => 'key-2'), false);
 assert.equal(unknownState.batch.status, 'outcome_unknown');
 const requestCountBeforeRetry = requests.length;
 assert.equal(await submitBlindBatch(unknownState, request, 'open', () => {}, () => 'key-3'), false);
 assert.equal(requests.length, requestCountBeforeRetry);
 assert.equal(await checkBlindBatch(unknownState, request, () => {}), true);
 assert.equal(requests.at(-1)[1].headers['Idempotency-Key'], 'key-2');
 assert.equal(JSON.parse(requests.at(-1)[1].body).action, 'close');
 assert.equal(unknownState.batch.id, 'batch-1');
});

test('batch polling preserves an unknown outcome for a known batch ID and updates terminal results', async () => {
 const {checkBlindBatch} = await import('../app.mjs');
 const state = {batch:{id:'batch-9', action:'close', status:'running', data:{status:'running'}}};
 let refreshPath = '';
 assert.equal(await checkBlindBatch(state, async path => { refreshPath = path; throw new Error('Temporary failure'); }, () => {}), false);
 assert.equal(refreshPath, '/api/control/v1/batches/batch-9');
 assert.equal(state.batch.status, 'outcome_unknown');
 assert.match(state.batch.error, /Temporary failure/);
 assert.equal(await checkBlindBatch(state, async path => ({id:'batch-9', action:'close', status:'partial', target_count:2, acknowledged_count:1, failed_count:1, pending_count:0, skipped_count:0, skipped:[], targets:[]}), () => {}), true);
 assert.equal(state.batch.status, 'partial');
 assert.equal(state.batch.data.acknowledged_count, 1);
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

test('all-blinds eligibility follows fresh availability, cooldown, and calibration state',()=>{
 const now=Date.parse('2026-11-05T23:00:00Z');
 const managed=[
  {availability:'ready'},
  {availability:'ready',gatt_readiness:{cooldown_until:'2026-11-05T23:01:00Z'}},
  {availability:'ready',gatt_readiness:{cooldown_until:'2026-11-05T22:59:00Z'}},
  {availability:'ready',calibration_mode_active:true},
  {availability:'stale'},
 ];
 assert.equal(batchEligibleCount(managed,now),2);
});

test('command messages distinguish acknowledgement and uncertain outcomes',()=>{
 assert.match(commandMessage({status:'write_acknowledged'})[0],/acknowledged/i);
 assert.match(commandMessage({status:'outcome_unknown'})[0],/check.*position/i);
 assert.match(commandMessage({status:'write_failed'})[0],/uncertain/i);
 assert.match(commandMessage({status:'connection_failed',retryable:true})[0],/no command was sent/i);
 assert.match(commandMessage({status:'connection_failed',failure_stage:'command_validation'})[0],/no bluetooth write was attempted/i);
 assert.match(commandMessage({status:'deferred'})[0],/cooldown/i);
});
test('command poll failure refreshes position before movement controls unlock', async()=>{
 const id='blind-1';
 const token=Symbol('command');
 const state={
  commands:{[id]:{status:'connecting',accepted_at:'2026-11-05T22:40:00.000Z'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  managed:[{enrollment:{id},position:{current_closed_percent:100}}],
  observations:[{id:'old-observation'}],
  actionError:null,
  serviceError:'Could not refresh diagnostics.',
  reconciliationRequired:new Set(),
 };
 const calls=[];
 const changes=[];
 const request=async path=>{
  calls.push(path);
  if(path==='/api/control/v1/devices')return [{enrollment:{id},availability:'ready',position:{current_closed_percent:95},position_freshness:'fresh',position_observed_at:'2026-11-05T22:40:01.000Z'}];
  if(path==='/api/v1/devices')return [{id:'fresh-observation'}];
  throw new Error(`Unexpected request: ${path}`);
 };

 const handled=await handleCommandPollFailure(
  state,
  id,
  token,
  new Error('Command was not found.'),
  ()=>refreshCommandTelemetry(state,request,id),
  ()=>changes.push({busy:state.busy.has(id),status:state.commands[id]?.status,position:state.managed[0]?.position?.current_closed_percent}),
 );

 assert.equal(handled,true);
 assert.deepEqual(calls,['/api/control/v1/devices','/api/v1/devices']);
 assert.deepEqual(changes,[
  {busy:true,status:'outcome_unknown',position:100},
  {busy:false,status:'outcome_unknown',position:95},
 ]);
 assert.equal(state.commands[id].status,'outcome_unknown');
 assert.equal(state.managed[0].position.current_closed_percent,95);
 assert.equal(state.observations[0].id,'fresh-observation');
 assert.equal(state.busy.has(id),false);
 assert.equal(state.reconciliationRequired.has(id),false);
 assert.match(state.actionError,/Could not check command status/);
 assert.match(state.actionError,/latest reported position/);
 assert.equal(state.serviceError,'Could not refresh diagnostics.');
});

test('failed managed-position refresh keeps movement locked until a later fresh refresh', async()=>{
 const id='blind-1';
 const token=Symbol('command');
 const state={
  commands:{[id]:{status:'connecting',accepted_at:'2026-11-05T22:40:00.000Z'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  managed:[{enrollment:{id},availability:'stale',position:{current_closed_percent:100},position_freshness:'stale',position_observed_at:'2026-11-05T22:39:00.000Z'}],
  observations:[{id:'old-observation'}],
  actionError:null,
  serviceError:'Unrelated service error.',
  reconciliationRequired:new Set(),
 };
 const request=async path=>{
  if(path==='/api/control/v1/devices')throw new Error('Managed-device request failed.');
  if(path==='/api/v1/devices')return [{id:'new-observation'}];
  throw new Error(`Unexpected request: ${path}`);
 };

 await handleCommandPollFailure(state,id,token,new Error('Command was not found.'),()=>refreshCommandTelemetry(state,request,id));

 assert.equal(state.busy.has(id),true);
 assert.equal(state.reconciliationRequired.has(id),true);
 assert.equal(state.managed[0].position.current_closed_percent,100);
 assert.equal(state.observations[0].id,'new-observation');
 assert.equal(state.serviceError,'Unrelated service error.');
 assert.match(state.actionError,/movement controls remain locked/);

 const fullRefresh=async path=>{
  if(path==='/api/control/v1/devices')return [{enrollment:{id},availability:'ready',position:{current_closed_percent:95},position_freshness:'fresh',position_observed_at:'2026-11-05T22:40:01.000Z'}];
  if(path==='/api/v1/devices')return [{id:'fresh-observation'}];
  if(path==='/api/v1/diagnostics')return {scanner:{adapter_available:true}};
  if(path==='/api/dev/simulators')return [];
  if(path==='/api/control/v1/audit-log')return [];
  throw new Error(`Unexpected request: ${path}`);
 };
 await refreshService(state,fullRefresh);

 assert.equal(state.reconciliationRequired.has(id),false);
 assert.equal(state.busy.has(id),false);
 assert.equal(state.managed[0].position.current_closed_percent,95);
 assert.match(state.actionError,/A fresh position is now available/);
});

test('fresh pre-command observation does not unlock an uncertain movement', async()=>{
 const id='blind-1';
 const token=Symbol('command');
 const state={
  commands:{[id]:{status:'connecting',accepted_at:'2026-11-05T22:40:00.000Z'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  managed:[],
  observations:[],
  actionError:null,
  serviceError:null,
  reconciliationRequired:new Set(),
 };
 const request=async path=>path==='/api/control/v1/devices'
  ? [{enrollment:{id},availability:'ready',position_freshness:'fresh',position_observed_at:'2026-11-05T22:39:59.999Z'}]
  : [];

 await handleCommandPollFailure(state,id,token,new Error('Command was not found.'),()=>refreshCommandTelemetry(state,request,id));

 assert.equal(state.busy.has(id),true);
 assert.equal(state.reconciliationRequired.has(id),true);
 assert.match(state.actionError,/fresh managed-device position is unavailable/);
});

test('stale command poll failure cannot clear a newer command state', async()=>{
 const id='blind-1';
 const token=Symbol('new command');
 let refreshed=false;
 const state={
  commands:{[id]:{status:'queued'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  actionError:null,
 };

 const handled=await handleCommandPollFailure(state,id,Symbol('old command'),new Error('Not found.'),async()=>{refreshed=true;});

 assert.equal(handled,false);
 assert.equal(refreshed,false);
 assert.equal(state.commands[id].status,'queued');
 assert.equal(state.busy.has(id),true);
 assert.equal(state.actionError,null);
});
test('fresh observations are not presented as GATT command readiness',()=>{
 assert.equal(availability('ready')[0], 'Observed recently');
 assert.equal(gattReadinessLabel({status:'untested'}), 'No command sent yet');
 assert.equal(gattReadinessLabel({status:'connection_failed'}), 'Could not connect; no write sent');
 assert.equal(gattReadinessLabel({status:'write_acknowledged'}), 'GATT write acknowledged');
 assert.equal(gattReadinessLabel({status:'outcome_unknown'}), 'Write outcome unknown');
 assert.equal(gattTransportEvidenceLabel('prewrite_failure'), 'The latest command failed before a write');
 assert.equal(gattTransportEvidenceLabel('recently_acknowledged'), 'A GATT write was recently acknowledged');
 assert.equal(motionFeedbackLabel({status:'stalled'}), 'No recent position progress');
 assert.equal(motionFeedbackLabel({status:'progress_observed'}), 'Position progress observed');
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

test('simulator enrollment limit explains how to add another later', async () => {
 const {enrollBlind} = await import('../app.mjs');
 const state = {enrollmentError:null};
 const request = async () => {
  const error = new Error('A maximum of two simulators can be enrolled at a time.');
  error.status = 409;
  error.code = 'simulator_limit_reached';
  throw error;
 };

 assert.equal(await enrollBlind(state, request, async () => {}, 'simulator-3', 'Simulator 3'), false);
 assert.match(state.enrollmentError, /You can add up to two simulators/);
 assert.match(state.enrollmentError, /Remove one from your managed blinds before adding another/);
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
