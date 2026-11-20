import {test} from 'node:test';
import assert from 'node:assert/strict';
import {activityMessage, availability, batchEligibleCount, buildCommandRequestBody, closedPercentFromOpenPercent, commandAwareMovementStatus, commandMessage, displayPreferenceFrom, escapeHtml, gattReadinessLabel, gattTransportEvidenceLabel, handleCommandPollFailure, isThemePreference, motionFeedbackLabel, nextDisplayPreference, observationFor, openPercentFromClosedPercent, positionCommandMessage, refreshCommandTelemetry, refreshService, relativeAt, resetDemo, resolveDisplayMode, shouldPauseRefresh, validateDisplayName} from '../app.mjs';
import type {ActivityEntry, ApiError, ApiRequest, BatchState, CommandRecord, DashboardState, ManagedDevice, Notification, RefreshTarget} from '../types.mjs';

type CommandTarget = RefreshTarget & {
  commands: Record<string, CommandRecord>;
  tokens: Record<string, symbol>;
  busy: Set<string>;
  reconciliationRequired: Set<string>;
};

function requestBody(options: RequestInit | undefined): Record<string, unknown> {
  return JSON.parse(String(options?.body ?? '{}')) as Record<string, unknown>;
}

function requestHeader(options: RequestInit | undefined, name: string): string | null {
  return new Headers(options?.headers).get(name);
}

test('automatic display mode distinguishes square panels from phones and desktop windows',()=>{
 assert.equal(resolveDisplayMode('auto', 800, 800), 'panel');
 assert.equal(resolveDisplayMode('auto', 390, 390), 'panel');
 assert.equal(resolveDisplayMode('auto', 390, 844), 'mobile');
 assert.equal(resolveDisplayMode('auto', 1440, 900), 'desktop');
 assert.equal(resolveDisplayMode('auto', 800, 560), 'desktop');
});

test('service worker reloads only after a user-requested waiting worker activation',async()=>{
 const {createServiceWorkerUpdateHandler}=await import('../app.mjs') as unknown as {
  createServiceWorkerUpdateHandler:(reload:()=>void)=>{
   requestActivation:(worker:Pick<ServiceWorker,'postMessage'>|null|undefined)=>void;
   onControllerChange:()=>void;
  };
 };
 let reloads=0;
 const handler=createServiceWorkerUpdateHandler(()=>{reloads++;});

 handler.onControllerChange();
 assert.equal(reloads,0);
 handler.requestActivation(null);
 handler.onControllerChange();
 assert.equal(reloads,0);

 const messages:unknown[]=[];
 const waitingWorker={postMessage:(message:unknown)=>messages.push(message)} as unknown as Pick<ServiceWorker,'postMessage'>;
 handler.requestActivation(waitingWorker);
 assert.deepEqual(messages,[{type:'SKIP_WAITING'}]);
 handler.onControllerChange();
 assert.equal(reloads,1);
 handler.onControllerChange();
 assert.equal(reloads,1);
});

test('availability and relative timestamps use plain language',()=>{
 assert.deepEqual((['ready','stale','not_seen','ambiguous','unavailable'] as const).map(value=>availability(value)[0]),[
  'Online','Not responding','Not found','Needs attention','Unavailable',
 ]);
 assert.equal(relativeAt('2026-11-16T11:57:00.000Z',Date.parse('2026-11-16T12:00:00.000Z')),'Last heard from 3 min ago');
});

test('refresh pauses while a drag, focused input, or confirmation is active',()=>{
 assert.equal(shouldPauseRefresh({dragging:true}),true);
 assert.equal(shouldPauseRefresh({focusedTag:'INPUT'}),true);
 assert.equal(shouldPauseRefresh({focusedTag:'SELECT'}),true);
 assert.equal(shouldPauseRefresh({confirmationOpen:true}),true);
 assert.equal(shouldPauseRefresh({focusedTag:'BUTTON'}),false);
 assert.equal(shouldPauseRefresh({}),false);
});

test('display name validation reports required and maximum length failures',()=>{
 assert.equal(validateDisplayName(''), 'Enter a name for this blind.');
 assert.equal(validateDisplayName('  '), 'Enter a name for this blind.');
 assert.equal(validateDisplayName('x'.repeat(65)), 'Use 64 characters or fewer.');
 assert.equal(validateDisplayName(' Living Room '), '');
});

test('room names are trimmed, capped at forty characters, and deduplicated without case',async()=>{
 const {roomNamesFromDevices,validateRoomName}=await import('../app.mjs') as unknown as {
  roomNamesFromDevices:(devices:ManagedDevice[])=>string[];
  validateRoomName:(value:string)=>string;
 };
 assert.equal(validateRoomName('  '),'Enter a room name.');
 assert.equal(validateRoomName('x'.repeat(41)),'Use 40 characters or fewer.');
 assert.equal(validateRoomName(' Unassigned '),'Choose another name; Unassigned is reserved.');
 assert.equal(validateRoomName('Study'),'');
 assert.deepEqual(roomNamesFromDevices([
  {enrollment:{id:'one',room:'Living Room'}},
  {enrollment:{id:'two',room:'living room'}},
  {enrollment:{id:'three',room:'Study'}},
  {enrollment:{id:'four'}},
 ]),['Living Room','Study']);
});

test('demo readiness stays positive while the physical adapter remains unavailable', async () => {
 const state: RefreshTarget & {demoMode:boolean; serviceError:string|null} = {demoMode:false,serviceError:null};
 const request: ApiRequest = async path => {
  if(path==='/api/control/v1/devices')return [];
  if(path==='/api/v1/devices')return [];
  if(path==='/api/v1/diagnostics')return {demo_mode:true,simulator_ready:true,scanner:{adapter_available:false,status:'unavailable'}};
  if(path==='/api/dev/simulators')return [];
  if(path==='/api/control/v1/audit-log')return [];
  throw new Error(`Unexpected path ${path}`);
 };
 await refreshService(state,request);
 assert.equal(state.demoMode,true);
 assert.equal(state.diagnostics?.simulator_ready,true);
 assert.equal(state.diagnostics?.scanner?.adapter_available,false);
 assert.equal(state.serviceError,null);
});

test('reset demo clears client command state and refreshes after confirmation', async () => {
 const messages: string[] = [];
 const target: RefreshTarget & {commands:Record<string,CommandRecord>;batch:BatchState|null;busy:Set<string>;notify:(item:{message:string})=>void} = {
  commands:{'demo-1':{status:'queued'}}, batch:{action:'open',status:'queued'}, busy:new Set(['demo-1']), notify:item=>messages.push(item.message),
 };
 const calls: string[] = [];
 const request: ApiRequest = async path => { calls.push(path); return {reset:true}; };
 let refreshed = false;
 assert.equal(await resetDemo(target,request,async()=>{refreshed=true;},()=>false),false);
 assert.deepEqual(calls,[]);
 assert.equal(await resetDemo(target,request,async()=>{refreshed=true;}),true);
 assert.deepEqual(calls,['/api/demo/reset']);
 assert.deepEqual(target.commands,{});
 assert.equal(target.batch,null);
 assert.equal(target.busy.size,0);
 assert.equal(refreshed,true);
 assert.match(messages.at(-1) ?? '',/reset to the original household/);
});

test('all-blinds batch submission is idempotent and a failed request stays locked for safe recovery', async () => {
 const {submitBlindBatch, checkBlindBatch} = await import('../app.mjs');
 const state: RefreshTarget & {batch: BatchState | null} = {batch:null};
 const requests: Array<[string, RequestInit | undefined]> = [];
 const request: ApiRequest = async (path, options) => {
  requests.push([path, options]);
  const action = options?.body ? JSON.parse(String(options.body)).action : 'open';
  return {id:'batch-1', action, status:'queued', target_count:2, acknowledged_count:0, failed_count:0, pending_count:2, skipped_count:0, skipped:[], targets:[]};
 };
 let changes = 0;
 assert.equal(await submitBlindBatch(state, request, 'open', () => { changes += 1; }, () => 'key-1'), true);
 assert.equal(requests.length, 1);
 assert.equal(requests[0][0], '/api/control/v1/batches');
 assert.equal(requestHeader(requests[0]?.[1], 'Idempotency-Key'), 'key-1');
 assert.deepEqual(requestBody(requests[0]?.[1]), {action:'open'});
 assert.equal(state.batch.id, 'batch-1');
 assert.equal(state.batch.status, 'queued');
 assert.equal(changes, 2);
 assert.equal(await submitBlindBatch(state, request, 'close', () => {}, () => 'key-2'), false);
 assert.equal(requests.length, 1);

 const unknownState: RefreshTarget & {batch: BatchState | null} = {batch:null};
 const failedRequest: ApiRequest = async (path, options) => {
  requests.push([path, options]);
  throw new Error('Network unavailable');
 };
 assert.equal(await submitBlindBatch(unknownState, failedRequest, 'close', () => {}, () => 'key-2'), false);
 assert.equal(unknownState.batch.status, 'outcome_unknown');
 const requestCountBeforeRetry = requests.length;
 assert.equal(await submitBlindBatch(unknownState, request, 'open', () => {}, () => 'key-3'), false);
 assert.equal(requests.length, requestCountBeforeRetry);
 assert.equal(await checkBlindBatch(unknownState, request, () => {}), true);
 assert.equal(requestHeader(requests.at(-1)?.[1], 'Idempotency-Key'), 'key-2');
 assert.equal(requestBody(requests.at(-1)?.[1]).action, 'close');
 assert.equal(unknownState.batch.id, 'batch-1');
});

test('targeted batch submission preserves the selected enrollment IDs and rejects an empty selection',async()=>{
 const {submitBlindBatch}=await import('../app.mjs');
 const state:RefreshTarget&{batch:BatchState|null}={batch:null};
 const requests:Array<RequestInit|undefined>=[];
 const request:ApiRequest=async(_path,options)=>{
  requests.push(options);
  return {id:'batch-room',action:'open',status:'queued',target_count:2};
 };
 assert.equal(await submitBlindBatch(state,request,'open',()=>{},()=> 'room-key',['blind-2','blind-1']),true);
 assert.deepEqual(requestBody(requests[0]),{action:'open',targets:['blind-2','blind-1']});
 assert.equal(state.batch?.targetIds?.length,2);
 const emptyState:RefreshTarget&{batch:BatchState|null}={batch:null};
 assert.equal(await submitBlindBatch(emptyState,request,'open',()=>{},()=> 'empty-key',[]),false);
});

test('batch polling preserves an unknown outcome for a known batch ID and updates terminal results', async () => {
 const {checkBlindBatch} = await import('../app.mjs');
 const state: RefreshTarget & {batch: BatchState | null; notifications: Notification[]} = {
  batch:{id:'batch-9', action:'close', status:'running', data:{status:'running'}},
  notifications:[],
  notify(item: Notification){this.notifications?.push(item);},
 };
 let refreshPath = '';
 assert.equal(await checkBlindBatch(state, async (path: string) => { refreshPath = path; throw new Error('Temporary failure'); }, () => {}), false);
 assert.equal(refreshPath, '/api/control/v1/batches/batch-9');
 assert.equal(state.batch.status, 'outcome_unknown');
 assert.match(state.batch.error, /Temporary failure/);
 assert.equal(await checkBlindBatch(state, async (_path: string) => ({id:'batch-9', action:'close', status:'partial', target_count:2, acknowledged_count:1, failed_count:1, pending_count:0, skipped_count:1, skipped:[{label:'Guest room',availability:'stale'}], targets:[{label:'Kitchen',status:'connection_failed'}]}), () => {}), true);
 assert.equal(state.batch.status, 'partial');
 assert.equal(state.batch.data.acknowledged_count, 1);
 assert.equal(state.notifications.at(-1).tone, 'warning');
 assert.match(state.notifications.at(-1).message, /1 of 2/);
 assert.match(state.notifications.at(-1).details.join(' '), /Guest room.*stale.*Kitchen.*connection failed/);
});

test('batch progress stays contextual while a terminal batch result becomes a snackbar', async () => {
 const {submitBlindBatch} = await import('../app.mjs');
 const notifications = [];
 const state: RefreshTarget & {batch: BatchState | null; notifications: Notification[]} = {batch:null, notifications, notify:item=>notifications.push(item)};
 const result = {id:'batch-done',action:'open',status:'completed',target_count:3,acknowledged_count:3,failed_count:0,pending_count:0,skipped_count:0,skipped:[],targets:[]};
 assert.equal(await submitBlindBatch(state,async()=>result,'open'),true);
 assert.equal(notifications.length,1);
 assert.equal(notifications[0].tone,'success');
 assert.match(notifications[0].message,/3 of 3/);
});

test('batch result notifications preserve failure details and surface rejected batches', async () => {
 const {submitBlindBatch} = await import('../app.mjs');
 const cases = [
  {
   result:{id:'batch-skipped',action:'close',status:'completed',target_count:2,acknowledged_count:2,failed_count:0,pending_count:0,skipped_count:1,skipped:[{label:'Office',availability:'stale'}],targets:[]},
   tone:'warning',
   message:/2 of 2.*1 unavailable/,
   detail:/Office.*stale/,
  },
  {
   result:{id:'batch-failed',action:'open',status:'failed',target_count:2,acknowledged_count:0,failed_count:2,pending_count:0,skipped_count:0,skipped:[],targets:[{label:'Kitchen',status:'connection_failed'},{label:'Bedroom',status:'outcome_unknown'}]},
   tone:'error',
   message:/No open commands were acknowledged; 2 failed/,
   detail:/Kitchen.*connection failed.*Bedroom.*outcome unknown/,
  },
  {
   result:{id:'batch-rejected',action:'close',status:'rejected',error:'The request was rejected.',target_count:0,acknowledged_count:0,failed_count:0,pending_count:0,skipped_count:0,skipped:[],targets:[]},
   tone:'error',
   message:/Could not start the close batch: The request was rejected\./,
   detail:null,
  },
 ] as const;

 for (const {result, tone, message, detail} of cases) {
  const notifications: Notification[] = [];
  const state: RefreshTarget & {batch:BatchState|null; notifications:Notification[]} = {
   batch:null,
   notifications,
   notify:item=>notifications.push(item),
  };
  assert.equal(await submitBlindBatch(state, async () => result, result.action), true);
  assert.equal(notifications[0]?.tone, tone);
  assert.match(notifications[0]?.message ?? '', message);
  if (detail) assert.match(notifications[0]?.details?.join(' ') ?? '', detail);
 }
});

test('action data parsing preserves encoded identifiers and ignores extra segments', async () => {
 const {parseActionValue} = await import('../app.mjs');
 assert.deepEqual(parseActionValue('command:open:blind%3Aleft'), ['command','open','blind%3Aleft']);
 assert.deepEqual(parseActionValue('details:blind%3Aleft'), ['details','blind%3Aleft','']);
 assert.deepEqual(parseActionValue(undefined), ['','','']);
 assert.deepEqual(parseActionValue('remove:blind-1:ignored'), ['remove','blind-1','ignored']);
});

test('theme settings accept the supported preferences and reject unknown values', () => {
 // Contract: UI-PREF-01 in docs/test-contracts.md.
 assert.deepEqual(['auto','dark','light','high-contrast'].map(isThemePreference),[true,true,true,true]);
 assert.equal(isThemePreference('unknown'),false);
});

test('command dispatch decodes the device ID before sending the command', async () => {
 const {dispatchBlindCommand} = await import('../app.mjs');
 const calls: Array<[string,string]> = [];
 assert.equal(dispatchBlindCommand('open','blind%3Aleft',(id,action)=>calls.push([id,action])),true);
 assert.deepEqual(calls,[['blind:left','open']]);
});

test('card notices retain full accessible details and escape regular-view text', async () => {
 const {cardMessageMarkup} = await import('../app.mjs');
 assert.equal(cardMessageMarkup(true,'','','','',''), '');

 const panel = cardMessageMarkup(
  true,
  'Command uncertain',
  'warn',
  'Controls unavailable',
  'Stop remains available until the cooldown ends',
  'Calibration is active',
 );
 assert.match(panel, /panel-notice__label">Movement paused/);
 assert.match(panel, /aria-label="Movement paused\. Command uncertain\. Controls unavailable\. Stop remains available until the cooldown ends\. Calibration is active/);

 const regular = cardMessageMarkup(false,'Could not display <Blind>','bad','','','');
 assert.match(regular, /&lt;Blind&gt;/);
 assert.doesNotMatch(regular, /<Blind>/);
});

test('device detail fields show cached telemetry age without jargon and only active cooldowns', async () => {
 const {deviceDetailFields} = await import('../app.mjs');
 const now = Date.parse('2026-11-15T17:00:00Z');
 const device: ManagedDevice = {
  enrollment:{id:'blind-1',label:'Office',room:'Study',is_simulated:true},
  availability:'ready',
  position:{current_closed_percent:42,target_closed_percent:60},
  last_seen_at:'2026-11-15T16:59:00Z',
  advertisement_age_ms:9500,
  motion_feedback:{status:'progress_observed',estimated_travel_rate_percent_per_second:0.75},
  gatt_readiness:{
   status:'connection_failed',transport_evidence:'prewrite_failure',
   last_attempt_at:'2026-11-15T16:58:00Z',last_write_acknowledged_at:null,
   cooldown_until:'2026-11-15T17:01:00Z',error_code:'connection_timeout',failure_stage:'connect',
  },
 };
 const observation = {
  movement_status:'closing' as const,
  rssi_dbm:-65,
  battery:{percent_estimate:80,voltage_mv:4000,status:'cached',observed_at:'2026-11-15T15:00:00Z'},
 };
 const fields = deviceDetailFields(device,observation,now);
 assert.deepEqual(fields.map(field=>field.label),[
  'Availability','Position','Last heard from','Advertisement age','Reported target','Movement feedback',
  'Learned travel rate','Signal strength','Battery','Battery voltage','Source','Last GATT command',
  'Transport evidence','Last command attempt','Last acknowledged GATT write','Movement cooldown until','Latest failure',
 ]);
 assert.equal(fields.find(field=>field.label==='Battery')?.value,'~80% · Last reading 2 h ago');
 assert.equal(fields.find(field=>field.label==='Battery voltage')?.value,'4000 mV · Last reading 2 h ago');
 assert.doesNotMatch(String(fields.find(field=>field.label==='Battery voltage')?.value),/cached/i);
 assert.equal(fields.find(field=>field.label==='Battery voltage')?.advanced,true);
 assert.equal(fields.find(field=>field.label==='Availability')?.value,'Online');
 assert.notEqual(fields.find(field=>field.label==='Signal strength')?.advanced,true);
 assert.equal(fields.find(field=>field.label==='Learned travel rate')?.value,'0.75% per second');
 assert.equal(fields.find(field=>field.label==='Movement cooldown until')?.value,new Date('2026-11-15T17:01:00Z').toLocaleString());

 const expired = deviceDetailFields({
  ...device,
  gatt_readiness:{...device.gatt_readiness,cooldown_until:'2026-11-15T16:59:00Z'},
 },observation,now);
 assert.equal(expired.some(field=>field.label==='Movement cooldown until'),false);
});

test('managed device cards preserve targets, status details, and panel safety states', async () => {
 const {renderManagedDeviceCard} = await import('../app.mjs');
 const now=Date.parse('2026-11-15T17:00:00Z');
 const dashboard: DashboardState = {
  demoMode:false,managed:[],observations:[],diagnostics:null,activity:[],commands:{},batch:null,
  serviceError:null,confirmingRemovalId:null,pendingRemovalFocusId:null,removingEnrollmentId:null,
  busy:new Set(),reconciliationRequired:new Set(),tokens:{},simulators:null,
  notify:()=>{},dismissNotification:()=>{},
 };
 const device: ManagedDevice = {
  enrollment:{id:'blind-1',logical_id:'logical-1',advertised_name:'Study shade',label:'Study shade',room:'Study',is_simulated:true},
  availability:'ready',position:{current_closed_percent:42,target_closed_percent:60},
  last_seen_at:'2026-11-15T16:59:00Z',capabilities:{set_position:true},
 };
 const secondDevice:ManagedDevice={...device,enrollment:{...device.enrollment,id:'blind-2',logical_id:'logical-2',label:'Bedroom shade'}};
 dashboard.managed=[device,secondDevice];
 dashboard.observations=[{
  id:'observation-1',logical_id:'logical-1',advertised_name:'Study shade',is_simulated:true,
  movement_status:'closing',battery:{percent_estimate:80,voltage_mv:4000,status:'observed'},
 }];
 const regular=renderManagedDeviceCard(device,dashboard,false,now);
 assert.match(regular,/Study shade/);
 assert.match(regular,/Drag the blind to set position/);
 assert.match(regular,/blind-state__label">Closing/);
 assert.match(regular,/data-position-preset="blind-1:50"/);
 assert.match(regular,/data-icon="battery"/);
 assert.match(regular,/>~80%<\/span>/);
 assert.doesNotMatch(regular,/Last heard from/);
 assert.doesNotMatch(renderManagedDeviceCard(secondDevice,dashboard,false,now),/Drag the blind to set position/);

 dashboard.commands['blind-1']={status:'connecting'};
 const connecting=renderManagedDeviceCard(device,dashboard,false,now);
 assert.match(connecting,/Sending command to the blind/);
 assert.doesNotMatch(connecting,/command-progress|data-command-progress/);
 delete dashboard.commands['blind-1'];
 const reached=renderManagedDeviceCard({...device,motion_feedback:{status:'target_reached'}},dashboard,false,now);
 assert.doesNotMatch(reached,/command-progress|Motor reports the target position/);

 dashboard.commands['blind-1']={status:'outcome_unknown'};
 const panel=renderManagedDeviceCard({
  ...device,availability:'stale',calibration_mode_active:true,
  gatt_readiness:{cooldown_until:'2026-11-15T17:01:00Z'},
 },dashboard,true,now);
 assert.match(panel,/panel-notice__label">Movement paused/);
 assert.match(panel,/aria-label="Movement paused\. The outcome is uncertain/);
 assert.match(panel,/Controls are unavailable until this blind is uniquely observed again/);
 assert.match(panel,/Endpoint calibration is active/);
 assert.doesNotMatch(panel,/position-control/);
 assert.match(panel,/data-action="command:open:blind-1"[^>]*disabled/);
});

test('display preference query pins a mode and otherwise uses the saved choice',()=>{
 assert.equal(displayPreferenceFrom('?view=panel', 'desktop'), 'panel');
 assert.equal(displayPreferenceFrom('?view=unknown', 'mobile'), 'mobile');
 assert.equal(displayPreferenceFrom('', 'panel'), 'panel');
 assert.equal(displayPreferenceFrom('', 'unknown'), 'auto');
});

test('display mode values remain ordered for the display picker',()=>{
 assert.equal(nextDisplayPreference('auto'), 'mobile');
 assert.equal(nextDisplayPreference('mobile'), 'panel');
 assert.equal(nextDisplayPreference('panel'), 'desktop');
 assert.equal(nextDisplayPreference('desktop'), 'auto');
 assert.equal(nextDisplayPreference('unknown'), 'auto');
});

test('explicit display mode overrides automatic viewport selection',()=>{
 assert.equal(resolveDisplayMode('panel', 390, 844), 'panel');
 assert.equal(resolveDisplayMode('mobile', 1440, 900), 'mobile');
 assert.equal(resolveDisplayMode('desktop', 390, 844), 'desktop');
 assert.equal(resolveDisplayMode('unknown', 390, 844), 'mobile');
});

test('all-blinds eligibility follows fresh availability, cooldown, and calibration state',()=>{
 const now=Date.parse('2026-11-05T23:00:00Z');
 const managed: Parameters<typeof batchEligibleCount>[0]=[
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

test('Stop commands expose queued and stopping states without overruling acknowledged telemetry',()=>{
 assert.equal(commandAwareMovementStatus('opening',{action:'stop',status:'queued'}),'stop_queued');
 assert.equal(commandAwareMovementStatus('opening',{action:'stop',status:'connecting'}),'stopping');
 assert.equal(commandAwareMovementStatus('opening',{action:'stop',status:'write_acknowledged'}),'opening');
 assert.equal(commandAwareMovementStatus('opening',{action:'open',status:'connecting'}),'opening');
 assert.match(commandMessage({action:'stop',status:'queued'})[0],/stop queued/i);
 assert.match(commandMessage({action:'stop',status:'connecting'})[0],/sending stop command/i);
 assert.match(commandMessage({action:'stop',status:'write_acknowledged'})[0],/stop command acknowledged.*position will update/i);
});

test('physical command payloads carry the label from the current managed-device snapshot',()=>{
 const device={enrollment:{id:'blind-1',label:'Hallway Blinds'}} as ManagedDevice;
 assert.deepEqual(buildCommandRequestBody(device,'open'),{action:'open',expected_label:'Hallway Blinds'});
 assert.deepEqual(buildCommandRequestBody(device,'set_position',2),{action:'set_position',expected_label:'Hallway Blinds',position_closed_percent:2});
 assert.deepEqual(buildCommandRequestBody(device,'set_motor_speed',undefined,37),{action:'set_motor_speed',expected_label:'Hallway Blinds',motor_speed_percent:37});
 assert.throws(()=>buildCommandRequestBody({enrollment:{id:'blind-1'}} as ManagedDevice,'close'),/Refresh the device list/);
});

test('percent-open controls convert to the unchanged percent-closed command contract',()=>{
 assert.equal(openPercentFromClosedPercent(0),100);
 assert.equal(openPercentFromClosedPercent(25),75);
 assert.equal(openPercentFromClosedPercent(100),0);
 assert.equal(closedPercentFromOpenPercent(0),100);
 assert.equal(closedPercentFromOpenPercent(25),75);
 assert.equal(closedPercentFromOpenPercent(100),0);
 assert.equal(positionCommandMessage(50),'Moving to 50% open…');
});
test('command poll failure refreshes position before movement controls unlock', async()=>{
 const id='blind-1';
 const token=Symbol('command');
 const state: CommandTarget={
  commands:{[id]:{status:'connecting',accepted_at:'2026-11-05T22:40:00.000Z'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  managed:[{enrollment:{id},position:{current_closed_percent:100}}],
  observations:[{id:'old-observation'}],
  notifications:[],
  notify(item: Notification){this.notifications?.push(item);},
  serviceError:'Could not refresh diagnostics.',
  reconciliationRequired:new Set(),
 };
 const calls: string[]=[];
 const changes: Array<{busy:boolean;status:string | undefined;position:number | undefined}> = [];
 const request: ApiRequest=async path=>{
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
 assert.match(state.notifications.at(-1).message,/Could not check command status/);
 assert.match(state.notifications.at(-1).message,/latest reported position/);
 assert.equal(state.notifications.at(-1).tone,'warning');
 assert.equal(state.serviceError,'Could not refresh diagnostics.');
});

test('failed managed-position refresh keeps movement locked until a later fresh refresh', async()=>{
 const id='blind-1';
 const token=Symbol('command');
 const state: CommandTarget={
  commands:{[id]:{status:'connecting',accepted_at:'2026-11-05T22:40:00.000Z'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  managed:[{enrollment:{id},availability:'stale',position:{current_closed_percent:100},position_freshness:'stale',position_observed_at:'2026-11-05T22:39:00.000Z'}],
  observations:[{id:'old-observation'}],
  notifications:[],
  notify(item: Notification){this.notifications?.push(item);},
  serviceError:'Unrelated service error.',
  reconciliationRequired:new Set(),
 };
 const request: ApiRequest=async path=>{
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
 assert.match(state.notifications.at(-1).message,/movement controls remain locked/);

 const fullRefresh: ApiRequest=async path=>{
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
 assert.equal(state.notifications.at(-1).message,'A fresh position is available. Verify it before trying again.');
});

test('fresh pre-command observation does not unlock an uncertain movement', async()=>{
 const id='blind-1';
 const token=Symbol('command');
 const state: CommandTarget={
  commands:{[id]:{status:'connecting',accepted_at:'2026-11-05T22:40:00.000Z'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  managed:[],
  observations:[],
  notifications:[],
  notify(item: Notification){this.notifications?.push(item);},
  serviceError:null,
  reconciliationRequired:new Set(),
 };
 const request: ApiRequest=async path=>path==='/api/control/v1/devices'
  ? [{enrollment:{id},availability:'ready',position_freshness:'fresh',position_observed_at:'2026-11-05T22:39:59.999Z'}]
  : [];

 await handleCommandPollFailure(state,id,token,new Error('Command was not found.'),()=>refreshCommandTelemetry(state,request,id));

 assert.equal(state.busy.has(id),true);
 assert.equal(state.reconciliationRequired.has(id),true);
 assert.match(state.notifications.at(-1).message,/fresh managed-device position is unavailable/);
});

test('stale command poll failure cannot clear a newer command state', async()=>{
 const id='blind-1';
 const token=Symbol('new command');
 let refreshed=false;
 const state: CommandTarget={
  commands:{[id]:{status:'queued'}},
  busy:new Set([id]),
  tokens:{[id]:token},
  reconciliationRequired:new Set(),
  notifications:[],
  notify(item: Notification){this.notifications?.push(item);},
 };

 const handled=await handleCommandPollFailure(state,id,Symbol('old command'),new Error('Not found.'),async()=>{refreshed=true;});

 assert.equal(handled,false);
 assert.equal(refreshed,false);
 assert.equal(state.commands[id].status,'queued');
 assert.equal(state.busy.has(id),true);
 assert.equal(state.notifications.length,0);
});
test('fresh observations are not presented as GATT command readiness',()=>{
 assert.equal(availability('ready')[0], 'Online');
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
 const calls: string[] = [];
 const request: ApiRequest = async path => {
  calls.push(path);
  if (path === '/api/dev/simulators') return simulators;
  if (path === '/api/v1/diagnostics') return {scanner:{adapter_available:true}};
  return [];
 };
 const state: RefreshTarget = {};
 await refreshService(state, request);
 assert.ok(calls.includes('/api/dev/simulators'));
 assert.deepEqual(state.simulators, simulators);
 assert.equal(state.serviceError, null);
});

test('service refresh loads recent activity and activity descriptions stay readable', async () => {
 const {refreshService} = await import('../app.mjs');
 const activity: ActivityEntry[] = [{kind:'command', action:'close', subject:'Study blind', status:'write_acknowledged'}];
 const calls: string[] = [];
 const request: ApiRequest = async path => {
  calls.push(path);
  if (path === '/api/control/v1/audit-log') return activity;
  if (path === '/api/v1/diagnostics') return {scanner:{adapter_available:true}};
  return [];
 };
 const state: RefreshTarget = {};
 await refreshService(state, request);
 assert.ok(calls.includes('/api/control/v1/audit-log'));
 assert.deepEqual(state.activity, activity);
 assert.equal(activityMessage(activity[0]), 'Close command · write acknowledged · Study blind');
 assert.equal(activityMessage({kind:'enrollment_removed', subject:'Study blind'}), 'Removed Study blind');
});

test('service refresh errors notify once per outage and announce recovery', async () => {
 const {refreshService} = await import('../app.mjs');
 const notices: Notification[] = [];
 const dismissed: string[] = [];
 const state: RefreshTarget = {notify:item=>notices.push(item),dismissNotification:key=>dismissed.push(key)};
 const failed: ApiRequest = async () => { throw new Error('Network unavailable'); };
 await refreshService(state, failed);
 await refreshService(state, failed);
 assert.equal(notices.length,1);
 assert.match(notices[0].message,/Network unavailable/);
 assert.equal(notices[0].key,'service-refresh');

 const recovered: ApiRequest = async path => path.endsWith('/diagnostics') ? {scanner:{adapter_available:true}} : [];
 await refreshService(state, recovered);
 assert.deepEqual(dismissed,['service-refresh']);
 assert.equal(notices.length,2);
 assert.equal(notices[1].tone,'success');
 assert.match(notices[1].message,/connection to Somaris is back/i);
});

test('enrollment errors notify in context and successful enrollment reports completion', async () => {
 const {enrollBlind} = await import('../app.mjs');
 const notices: Notification[] = [];
 const state: RefreshTarget = {notify:item=>notices.push(item)};
 const request: ApiRequest = async (path, options) => {
  if (options?.method === 'POST') throw new Error('Device is no longer fresh');
  return {};
 };
 const refresh = async () => {};
 assert.equal(await enrollBlind(state, request, refresh, 'blind-1', 'Hallway'), false);
 let sentBody: Record<string, unknown> = {};
 const successfulRequest: ApiRequest = async (path, options) => {
  if(options?.method==='POST')sentBody=requestBody(options);
  return options?.method === 'POST' ? {} : request(path, options);
 };
 assert.equal(await enrollBlind(state, successfulRequest, refresh, 'blind-1', 'Hallway', 'Study'), true);
 assert.deepEqual(sentBody,{observed_device_id:'blind-1',label:'Hallway',room:'Study'});
 assert.equal(notices.length,2);
 assert.equal(notices[0].tone,'error');
 assert.match(notices[0].message,/Device is no longer fresh/);
 assert.equal(notices[1].tone,'success');
 assert.match(notices[1].message,/Hallway.*added/i);
});

test('room assignment uses the room API, refreshes managed state, and keeps errors visible',async()=>{
 const {updateBlindRoom}=await import('../app.mjs') as unknown as {
  updateBlindRoom:(target:RefreshTarget,request:ApiRequest,refresh:()=>Promise<unknown>,id:string,room:string|null)=>Promise<boolean>;
 };
 const notices:Notification[]=[];
 const calls:Array<[string,RequestInit|undefined]> = [];
 let refreshed=0;
 const state:RefreshTarget={notify:item=>notices.push(item)};
 const request:ApiRequest=async(path,options)=>{calls.push([path,options]);return {};};
 assert.equal(await updateBlindRoom(state,request,async()=>{refreshed++;},'managed/a','Living Room'),true);
 assert.equal(calls[0]?.[0],'/api/control/v1/devices/managed%2Fa');
 assert.equal(calls[0]?.[1]?.method,'PATCH');
 assert.deepEqual(requestBody(calls[0]?.[1]),{room:'Living Room'});
 assert.equal(refreshed,1);
 assert.equal(notices.at(-1)?.tone,'success');

 const failing:ApiRequest=async()=>{throw new Error('Room name already exists');};
 assert.equal(await updateBlindRoom(state,failing,async()=>{},'managed/a',null),false);
 assert.equal(notices.at(-1)?.tone,'error');
 assert.match(notices.at(-1)?.message||'',/Room name already exists/);
});

test('battery display shows estimated percentage and the age of the last reading', async () => {
 const {batteryDisplay} = await import('../app.mjs');
 const observed = batteryDisplay({status:'observed', percent_estimate:76, voltage_mv:3980, observed_at:'2026-11-19T10:00:00.000Z'},Date.parse('2026-11-19T12:00:00.000Z'));
 assert.match(observed, />~76%<\/span>/);
 assert.match(observed, /Last reading/);
 assert.match(observed, /2 h ago/);
 assert.match(observed, /data-icon="battery"/);
 assert.doesNotMatch(observed, /popover|aria-haspopup|cached/i);
 assert.doesNotMatch(observed, /cached/);
 assert.match(batteryDisplay({status:'unavailable', percent_estimate:0, voltage_mv:3600}), />~0%<\/span>/);
 assert.doesNotMatch(batteryDisplay({status:'unavailable', percent_estimate:0, voltage_mv:3600}), /cached/i);
 assert.match(batteryDisplay({status:'observed', percent_estimate:100}), />~100%<\/span>/);
 assert.match(batteryDisplay({status:'observed', voltage_mv:4120}), /Battery unavailable/);
 assert.match(batteryDisplay(null), /Battery unavailable/);
});

test('simulator enrollment limit explains how to add another later', async () => {
 const {enrollBlind} = await import('../app.mjs');
 const notices: Notification[] = [];
 const state: RefreshTarget = {notify:item=>notices.push(item)};
 const request = async () => {
  const error = Object.assign(new Error('A maximum of two simulators can be enrolled at a time.'), {
   status:409,
   code:'simulator_limit_reached',
  });
  throw error;
 };

 assert.equal(await enrollBlind(state, request, async () => {}, 'simulator-3', 'Simulator 3'), false);
 assert.match(notices[0].message, /You can add up to two simulators/);
 assert.match(notices[0].message, /Remove one from your managed blinds before adding another/);
});

test('removing an enrollment sends DELETE, clears local device state, and refreshes', async () => {
 const {removeEnrolledBlind} = await import('../app.mjs');
 const id = 'managed/a b';
 const token = Symbol('command');
 const notices: Notification[]=[];
 const state: RefreshTarget & {
  managed: ManagedDevice[];
  commands: Record<string, CommandRecord>;
  busy: Set<string>;
  tokens: Record<string, symbol>;
  confirmingRemovalId: string | null;
  removingEnrollmentId: string | null;
 } = {
  managed:[{enrollment:{id}},{enrollment:{id:'other'}}],
  commands:{[id]:{status:'write_acknowledged'}},
  busy:new Set([id]), tokens:{[id]:token},
  confirmingRemovalId:id,
  removingEnrollmentId:null,
  notify:item=>notices.push(item),
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
 assert.equal(notices.length,1);
 assert.equal(notices[0].tone,'success');
 assert.match(notices[0].message,/removed/i);
 assert.equal(refreshes, 1);
});

test('removal failures remain visible and keep the confirmation available for retry', async () => {
 const {removeEnrolledBlind} = await import('../app.mjs');
 const id = 'managed-1';
 const notices: Notification[] = [];
 const state: RefreshTarget & {
  managed: ManagedDevice[];
  confirmingRemovalId: string | null;
  removingEnrollmentId: string | null;
 } = {managed:[{enrollment:{id}}], confirmingRemovalId:id, removingEnrollmentId:null, notify:item=>notices.push(item)};
 let refreshes = 0;
 const request = async () => { throw new Error('The enrollment has queued or running commands.'); };

 assert.equal(await removeEnrolledBlind(state, request, async () => { refreshes += 1; }, id), false);
 assert.match(notices[0].message, /queued or running commands/);
 assert.match(notices[0].message, /refresh/i);
 assert.equal(notices[0].tone,'error');
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
