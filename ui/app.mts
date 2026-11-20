import {
  escapeHtml,
  renderBlindActions,
  renderBlindCard,
  renderBlindStatus,
  renderIcon,
  renderButton,
  renderIconButton,
  renderDismissButton,
  renderDemoBanner,
  renderRoomGroup,
  renderRoomCollection,
  renderRoomPicker,
  roomFromChoice,
  renderServiceSummary,
  renderIconLinkButton,
  renderDestructiveButton,
  renderConfirmationPanel,
  renderDeviceList,
  renderTopbarActions,
  DISPLAY_MODE_OPTIONS,
  renderHomeSupportingSections,
  renderBatchControls,
  renderPanelNotice,
  renderInfoGrid,
  renderBatteryReading,
  renderRelativeTime,
  relativeTimeLabel,
  renderActivitySection,
  renderBackButton,
  renderSettingsPage,
  renderMotorSpeedControl,
  renderCalibrationControls,
  renderEmptyState,
  renderAddDeviceEmptyState,
  renderHomeContent,
  renderDisclosure,
  movementStatusFromPositions,
  renderMobileTabs,
  renderSnackbar,
  renderSnackbarRegion,
} from './components.mjs';
import {SnackbarQueue} from './snackbar.mjs';
import {mountClock} from './clock.mjs';
import type {
  ActivityEntry,
  ApiError,
  ApiRequest,
  Availability as AvailabilityStatus,
  BatchAction,
  BatchResponse,
  BatchState,
  CommandRecord,
  DashboardState,
  Diagnostics,
  DisplayPreference,
  GattReadiness,
  ManagedDevice,
  MotionFeedback,
  Notification,
  Observation,
  ObservationIdentity,
  RefreshTarget,
  Simulator,
  SnackbarItem,
  Tone,
  VisualTone,
} from './types.mjs';

export {escapeHtml};
const displayModes = new Set<DisplayPreference>(DISPLAY_MODE_OPTIONS.map(([value]) => value));
const themeModes = new Set(['auto','dark','light','high-contrast']);
export function isThemePreference(preference:string):boolean { return themeModes.has(preference); }
const displayPreferenceCycle: DisplayPreference[] = DISPLAY_MODE_OPTIONS.map(([value]) => value);
export function nextDisplayPreference(preference: string): DisplayPreference {
  const index = displayPreferenceCycle.indexOf(preference as DisplayPreference);
  return displayPreferenceCycle[(index + 1 + displayPreferenceCycle.length) % displayPreferenceCycle.length] ?? 'auto';
}
export function resolveDisplayMode(preference: string, width: number, height: number): DisplayPreference {
  if (displayModes.has(preference as DisplayPreference) && preference !== 'auto') return preference as DisplayPreference;
  const viewportWidth = Number.isFinite(width) ? Math.max(0, width) : 0;
  const viewportHeight = Number.isFinite(height) ? Math.max(0, height) : 0;
  const aspectRatio = viewportHeight ? viewportWidth / viewportHeight : 0;
  if (viewportWidth > 0 && viewportHeight > 0 && Math.max(viewportWidth, viewportHeight) <= 1100 && aspectRatio >= 0.82 && aspectRatio <= 1.22) return 'panel';
  return viewportWidth <= 600 ? 'mobile' : 'desktop';
}
export function displayPreferenceFrom(search: string | null | undefined, saved: string | null | undefined): DisplayPreference {
  const requested = new URLSearchParams(search || '').get('view');
  if (requested && displayModes.has(requested as DisplayPreference)) return requested as DisplayPreference;
  return saved && displayModes.has(saved as DisplayPreference) ? saved as DisplayPreference : 'auto';
}
export function resolveActionFocus({
  focusedAction = null,
  pendingConfirmationId = null,
  confirmingRemovalId = null,
  availableActions = [],
}: {
  focusedAction?: string | null;
  pendingConfirmationId?: string | null;
  confirmingRemovalId?: string | null;
  availableActions?: string[];
}): string | null {
  if (pendingConfirmationId && pendingConfirmationId === confirmingRemovalId) {
    const cancelAction = `remove-cancel:${encodeURIComponent(pendingConfirmationId)}`;
    if (availableActions.includes(cancelAction)) return cancelAction;
  }
  if (focusedAction && availableActions.includes(focusedAction)) return focusedAction;
  if (focusedAction?.startsWith('remove-cancel:')) {
    const removeAction = `remove:${focusedAction.slice('remove-cancel:'.length)}`;
    if (availableActions.includes(removeAction)) return removeAction;
  }
  return null;
}
function storedDisplayPreference(): DisplayPreference {
  let saved;
  try {
    saved = globalThis.localStorage?.getItem('somaris-display-mode');
  } catch { /* Storage can be unavailable in private browsing contexts. */ }
  return displayPreferenceFrom(globalThis.location?.search, saved);
}
export function commandMessage(command: CommandRecord): [string, VisualTone] {
  switch (command.status) {
    case 'queued': return command.action === 'stop' ? ['Stop queued. Waiting for the blind…', ''] : ['Queued. Waiting for the blind…', ''];
    case 'connecting': return command.action === 'stop' ? ['Sending stop command to the blind…', ''] : ['Sending command to the blind…', ''];
    case 'deferred': return ['Movement paused by the Bluetooth cooldown. Stop remains available.', 'warn'];
    case 'write_acknowledged': return command.action === 'stop'
      ? ['Stop command acknowledged. Position will update when reported by the blind.', '']
      : ['Command acknowledged. Movement is shown when reported by the blind.', ''];
    case 'outcome_unknown': case 'write_failed': return ['The outcome is uncertain. Check the reported position before trying again.', 'warn'];
    case 'connection_failed':
      if (command.failure_stage === 'command_validation') return ['Command data is missing or invalid. No Bluetooth write was attempted.', 'bad'];
      return [command.retryable ? 'Could not connect. No command was sent; you can try again.' : 'Could not connect to the blind.', 'bad'];
    case 'expired': return ['The command expired before it could be sent.', 'bad'];
    case 'cancelled': return ['The command was cancelled.', 'warn'];
    default: return ['Checking command status…', ''];
  }
}
export function commandAwareMovementStatus(observed: NonNullable<ObservationIdentity['movement_status']>, command: CommandRecord | undefined): NonNullable<ObservationIdentity['movement_status']> | 'stop_queued' | 'stopping' {
 if(command?.action !== 'stop')return observed;
 if(command.status === 'queued')return 'stop_queued';
 if(command.status === 'connecting')return 'stopping';
 return observed;
}
export function activityMessage(activity: ActivityEntry): string {
  const subject = activity.subject || 'Somaris';
  const action = activity.action ? `${activity.action.charAt(0).toUpperCase()}${activity.action.slice(1)}` : '';
  const status = activity.status?.replaceAll('_', ' ');
  switch (activity.kind) {
    case 'enrollment_added': return `Added ${subject}`;
    case 'enrollment_removed': return `Removed ${subject}`;
    case 'room_assignment_changed': return `Updated room assignment · ${subject}`;
    case 'room_renamed': return `Renamed room · ${subject}`;
    case 'room_removed': return `Removed room · ${subject}`;
    case 'command': return `${action} command${status ? ` · ${status}` : ''} · ${subject}`;
    case 'batch': return `${action} batch${status ? ` · ${status}` : ''} · ${subject}`;
    case 'simulator_settings_changed': return `Updated simulator settings · ${subject}${activity.detail ? ` · ${activity.detail}` : ''}`;
    case 'simulator_reset': return `Reset simulator · ${subject}${activity.detail ? ` · ${activity.detail}` : ''}`;
    default: return `Changed ${subject}`;
  }
}
export function observationFor(
  managed: {enrollment: Pick<ManagedDevice['enrollment'], 'logical_id' | 'advertised_name' | 'is_simulated'>},
  observations: ObservationIdentity[],
): ObservationIdentity | null {
  const e = managed.enrollment;
  const matches = observations.filter(o => o.logical_id === e.logical_id && o.advertised_name === e.advertised_name && !!o.is_simulated === !!e.is_simulated);
  return matches.length === 1 ? matches[0] ?? null : null;
}
export function batteryDisplay(battery: ManagedDevice['battery'], now = Date.now()): string {
  return renderBatteryReading({battery,now});
}
const terminal = new Set<CommandRecord['status']>(['write_acknowledged','connection_failed','write_failed','outcome_unknown','expired','cancelled','deferred']);
const app = typeof document === 'undefined' ? null : document.querySelector<HTMLElement>('#app');
const snackbarRoot = typeof document === 'undefined' ? null : document.querySelector<HTMLElement>('#snackbar-root');
let renderSnackbars: (items: SnackbarItem[]) => void = () => {};
const snackbarQueue = new SnackbarQueue({onChange:items => renderSnackbars(items)});
const state: DashboardState = {demoMode:false, managed:[], observations:[], diagnostics:null, activity:[], commands:{}, batch:null, serviceError:null, confirmingRemovalId:null, pendingRemovalFocusId:null, removingEnrollmentId:null, busy:new Set(), reconciliationRequired:new Set(), tokens:{}, simulators:null, notify:item=>snackbarQueue.push(item), dismissNotification:key=>snackbarQueue.dismissByKey(key)};
let demoSessionRecovery: Promise<void> | null = null;
let displayPreference = storedDisplayPreference();
function storedPreference(key: string, fallback: string): string {
 try { return globalThis.localStorage?.getItem(key) || fallback; } catch { return fallback; }
}
function storedFlag(key: string): boolean {
 try { return globalThis.localStorage?.getItem(key) === 'true'; } catch { return false; }
}
let themePreference = storedPreference('somaris-theme','auto');
let hintDismissed = storedFlag('somaris-demo-hint-dismissed');
let positionCueDismissed = storedFlag('somaris-position-cue-dismissed');
let activityCollapsedByUser: boolean | null = null;
let editingRoomName: string | null = null;
let confirmingRoomRemoval: string | null = null;
let positionInteraction = false;
const positionStartValues = new Map<string,number>();
let positionInteractionId: string | null = null;
const lastPositionCommit = new Map<string,{value:number;at:number}>();
let positionUndo: {id:string; previous:number; commandToken:symbol; expires:number} | null = null;
const positionFeedback=new Map<string,{key:string;targetClosedPercent:number;openPercent:number;label:string;acceptedAt:number}>();
let batchUndo: {id:string; previous:Map<string,number>; expires:number} | null = null;
let lastRenderedRoute = typeof location === 'undefined' ? 'home' : location.hash.slice(1)||'home';
const routeScrollPositions = new Map<string,number>();
const esc = escapeHtml;
export function relativeAt(date: string | null | undefined, now: number): string {
 if(!date)return 'Never heard from';
 const seconds=Math.max(0,Math.floor((now-new Date(date).getTime())/1000));
 if(!Number.isFinite(seconds))return 'Time unavailable';
 if(seconds<60)return 'Last heard from just now';
 if(seconds<3600)return `Last heard from ${Math.floor(seconds/60)} min ago`;
 if(seconds<86400)return `Last heard from ${Math.floor(seconds/3600)} hr ago`;
 return `Last heard from ${Math.floor(seconds/86400)} days ago`;
}
export function openPercentFromClosedPercent(value:number):number {
 return 100-Math.max(0,Math.min(100,Math.round(value)));
}
export function closedPercentFromOpenPercent(value:number):number {
 return 100-Math.max(0,Math.min(100,Math.round(value)));
}
export function positionCommandMessage(openPercent:number):string {
 return `Moving to ${Math.max(0,Math.min(100,Math.round(openPercent)))}% open…`;
}
const relative = (date: string | null | undefined): string => relativeAt(date,Date.now());
function isRecord(value: unknown): value is Record<string, unknown> {
 return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function apiError(error: unknown): ApiError {
 if (error instanceof Error) return error as ApiError;
 return new Error(String(error));
}
function errorMessage(error: unknown): string {
 return error instanceof Error ? error.message : String(error);
}
async function api<T = unknown>(path: string, options: RequestInit = {}, recoverExpiredSession = true): Promise<T> {
 const response = await fetch(path, {cache:'no-store', credentials:'same-origin', ...options});
 const body: unknown = await response.json().catch(() => ({}));
 const envelope = isRecord(body) ? body : {};
 const errorValue = isRecord(envelope.error) ? envelope.error : {};
 if (!response.ok) {
  const error = new Error(String(errorValue.message ?? envelope.message ?? `Request failed (${response.status})`)) as ApiError;
  error.status = response.status;
  if (typeof errorValue.code === 'string') error.code = errorValue.code;
  if (recoverExpiredSession && state.demoMode && error.code === 'demo_session_expired' && path !== '/api/demo/session') {
   if (!demoSessionRecovery) {
    demoSessionRecovery = api('/api/demo/session', {method:'POST'}, false)
     .then(() => {
      state.commands = {}; state.batch = null; state.busy.clear(); state.reconciliationRequired.clear();
      snackbarQueue.push({message:'Your demo session expired. A fresh demonstration was started.',tone:'warning'});
     })
     .finally(() => { demoSessionRecovery = null; });
   }
   await demoSessionRecovery;
   if (!options.method || options.method.toUpperCase() === 'GET') return api<T>(path, options, false);
   error.message = 'Your demo session expired and a fresh demo was started. Review the reset demo before continuing.';
  }
  throw error;
 }
 return envelope.data as T;
}
function notifyTarget(target: RefreshTarget, notification: Notification): void {
 if(typeof target?.notify === 'function') target.notify(notification);
 else { target.notifications ??= []; target.notifications.push(notification); }
}
function dismissTargetNotification(target: RefreshTarget, key: string): void { target.dismissNotification?.(key); }
function serviceFailure(target: RefreshTarget, error: unknown): void {
 const message=`Could not refresh Somaris: ${errorMessage(error)}`;
 if(target.serviceError!==message){
  if(target.serviceError)dismissTargetNotification(target,'service-refresh');
  notifyTarget(target,{key:'service-refresh',message,tone:'error'});
 }
 target.serviceError=message;
}
function serviceRecovered(target: RefreshTarget): void {
 const recovered=!!target.serviceError;
 target.serviceError=null;
 if(recovered){
  dismissTargetNotification(target,'service-refresh');
  notifyTarget(target,{key:'service-restored',message:'The connection to Somaris is back.',tone:'success'});
 }
}
renderSnackbars = (items: SnackbarItem[]): void => {
 if(!snackbarRoot) return;
 const focused=snackbarRoot.contains(document.activeElement) ? document.activeElement as HTMLElement : null;
 const focusedId=focused?.closest<HTMLElement>('[data-snackbar-id]')?.dataset.snackbarId;
 const focusedControl=focused?.dataset.snackbarControl;
 if(!snackbarRoot.querySelector('#snackbar-region'))snackbarRoot.innerHTML=renderSnackbarRegion([]);
 const region=snackbarRoot.querySelector<HTMLElement>('#snackbar-region');
 if(!region) return;
 const visibleIds=new Set(items.map(item=>item.id));
 for(const item of [...region.querySelectorAll<HTMLElement>('[data-snackbar-id]')]){
  if(!visibleIds.has(item.dataset.snackbarId ?? ''))item.remove();
 }
 for(const item of items){
  if(region.querySelector(`[data-snackbar-id="${CSS.escape(item.id)}"]`))continue;
  const template=document.createElement('template');
  template.innerHTML=renderSnackbar(item);
  const snackbar=template.content.firstElementChild;
  if(snackbar)region.append(snackbar);
 }
 if(focusedId && focusedControl){
  const itemSelector=`[data-snackbar-id="${CSS.escape(focusedId)}"] [data-snackbar-control="${CSS.escape(focusedControl)}"]`;
  snackbarRoot.querySelector<HTMLElement>(itemSelector)?.focus({preventScroll:true});
 }
};
if(snackbarRoot) renderSnackbars(snackbarQueue.visibleItems);
const availabilityLabels: Record<AvailabilityStatus, [string, VisualTone]> = {
 ready:['Online',''], stale:['Not responding','warn'], not_seen:['Not found','warn'],
 ambiguous:['Needs attention','bad'], unavailable:['Unavailable','warn'],
};
export function availability(value: AvailabilityStatus | undefined): [string, VisualTone] {
 return value ? availabilityLabels[value] : availabilityLabels.unavailable;
}

export function shouldPauseRefresh({dragging = false, focusedTag = '', confirmationOpen = false}: {dragging?: boolean; focusedTag?: string; confirmationOpen?: boolean} = {}): boolean {
 return dragging || confirmationOpen || ['INPUT','SELECT','TEXTAREA'].includes(focusedTag.toUpperCase());
}

export function validateDisplayName(value: string): string {
 const label=value.trim();
 if(!label)return 'Enter a name for this blind.';
 if(label.length>64)return 'Use 64 characters or fewer.';
 return '';
}
export function validateRoomName(value: string): string {
 const room=value.trim();
 if(!room)return 'Enter a room name.';
 if([...room].length>40)return 'Use 40 characters or fewer.';
 if(room.toLowerCase()==='unassigned')return 'Choose another name; Unassigned is reserved.';
 return '';
}
export function roomNamesFromDevices(devices: Pick<ManagedDevice,'enrollment'>[]): string[] {
 const rooms=new Map<string,string>();
 for(const device of devices){
  const name=device.enrollment.room?.trim();
  if(name&&!rooms.has(name.toLowerCase()))rooms.set(name.toLowerCase(),name);
 }
 return [...rooms.values()].sort((left,right)=>left.localeCompare(right,undefined,{sensitivity:'base'}));
}
const terminalBatchStatuses = new Set(['completed', 'partial', 'failed', 'rejected']);
function blindBatchIsLocked(batch: BatchState | null | undefined): boolean {
 return !!batch && !terminalBatchStatuses.has(batch.status);
}
export function batchEligibleCount(managed: {availability?: string; calibration_mode_active?: boolean; gatt_readiness?: Pick<GattReadiness, 'cooldown_until'> | null}[], now = Date.now()): number {
 return (managed || []).filter(device => {
  if(device.availability!=='ready' || device.calibration_mode_active===true) return false;
  const cooldownUntil=Date.parse(device.gatt_readiness?.cooldown_until || '');
  return !Number.isFinite(cooldownUntil) || cooldownUntil<=now;
 }).length;
}
function newIdempotencyKey() {
 return globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}
function batchRequestOptions(batch: BatchState): RequestInit {
 const body:Record<string,unknown>={action:batch.action};
 if(batch.targetIds)body.targets=batch.targetIds;
 return {
  method:'POST',
  headers:{'Content-Type':'application/json','Idempotency-Key':batch.idempotencyKey ?? ''},
  body:JSON.stringify(body),
 };
}
const batchFailureStatuses = new Set(['connection_failed','deferred','write_failed','outcome_unknown','expired','cancelled']);

interface BatchNotificationCopy {
 message: string;
 tone: Tone;
}

function batchNotificationCopy(
 label: string,
 status: string | undefined,
 acknowledged: number,
 total: number,
 failed: number,
 skipped: number,
 error: string | null | undefined,
): BatchNotificationCopy | null {
 const unavailable = skipped ? `; ${skipped} unavailable` : '';
 switch (status) {
  case 'completed':
   return {
    message:`${label} commands acknowledged for ${acknowledged} of ${total} ready blinds${unavailable}. Acknowledgement does not confirm movement.`,
    tone:skipped ? 'warning' : 'success',
   };
  case 'partial':
   return {message:`${label} commands acknowledged for ${acknowledged} of ${total} ready blinds; ${failed} failed${unavailable}.`,tone:'warning'};
  case 'failed':
   return {message:`No ${label.toLowerCase()} commands were acknowledged; ${failed} failed${unavailable}.`,tone:'error'};
  case 'rejected':
   return {message:`Could not start the ${label.toLowerCase()} batch: ${error || 'The request was rejected.'}`,tone:'error'};
  default:
   return null;
 }
}

function batchNotificationDetails(data: Partial<BatchResponse> | BatchState): string[] {
 const skipped = (data.skipped || []).map(item => `Skipped ${item.label || 'blind'} · ${(item.availability || 'unavailable').replaceAll('_',' ')}`);
 const failures = (data.targets || [])
  .filter(target => batchFailureStatuses.has(target.status))
  .map(target => `${target.label || 'Blind'} · ${target.status.replaceAll('_',' ')}`);
 return [...skipped, ...failures];
}

function batchResultNotification(batch: BatchState): Notification | null {
 const data: Partial<BatchResponse> | BatchState=batch.data||batch;
 const label=(batch.action||data.action)==='close'?'Close':'Open';
 const status=batch.status||data.status;
 const skipped=data.skipped?.length ?? 0;
 const copy=batchNotificationCopy(
  label,
  status,
  data.acknowledged_count||0,
  data.target_count||0,
  data.failed_count||0,
  skipped,
  batch.error||data.error,
 );
 if(!copy)return null;
 const details=batchNotificationDetails(data);
 const visibleDetails=details.slice(0,4);
 if(details.length>visibleDetails.length)visibleDetails.push(`${details.length-visibleDetails.length} more details`);
 return {key:`batch:${batch.id||batch.idempotencyKey||'current'}:result`,...copy,details:visibleDetails};
}
function applyBatchResult(target: RefreshTarget & {batch: BatchState | null}, batch: BatchState, result: BatchResponse, onChange: () => void): boolean {
 if(target.batch!==batch) return false;
 batch.id=result.id || batch.id;
 batch.data=result;
 batch.status=result.status;
 batch.error=result.error||null;
 const notification=batchResultNotification(batch);
 if(notification){
  dismissTargetNotification(target,`batch:${batch.id||batch.idempotencyKey||'current'}:unknown`);
  notifyTarget(target,notification);
 }
 onChange();
 return true;
}
export async function submitBlindBatch(
 target: RefreshTarget & {batch: BatchState | null},
 request: ApiRequest,
 action: BatchAction,
 onChange: () => void = () => {},
 createKey: () => string = newIdempotencyKey,
 targetIds?:string[],
): Promise<boolean> {
 if(!['open','close'].includes(action) || blindBatchIsLocked(target.batch)) return false;
 if(targetIds&&(!targetIds.length||new Set(targetIds).size!==targetIds.length))return false;
 const batch: BatchState={action,status:'submitting',idempotencyKey:createKey(),id:null,data:null,error:null,targetIds:targetIds?[...targetIds]:undefined};
 target.batch=batch;
 onChange();
 try {
  const result=await request('/api/control/v1/batches',batchRequestOptions(batch));
  return applyBatchResult(target,batch,result as BatchResponse,onChange);
 } catch(error) {
  if(target.batch!==batch) return false;
  const failure=apiError(error);
  batch.error=String(failure.message || error);
  const statusCode = failure.status ?? 0;
  batch.status=statusCode>=400 && statusCode<500 ? 'rejected' : 'outcome_unknown';
  if(batch.status==='rejected'){
   const notification=batchResultNotification(batch);
   if(notification)notifyTarget(target,notification);
  } else {
   notifyTarget(target,{key:`batch:${batch.id||batch.idempotencyKey}:unknown`,message:`The ${action} batch result is unknown. Check its status before starting another batch.`,tone:'warning',action:{label:'Check status',action:'batch:check',icon:'refresh'}});
  }
  onChange();
  return false;
 }
}
export async function checkBlindBatch(
 target: RefreshTarget & {batch: BatchState | null},
 request: ApiRequest,
 onChange: () => void = () => {},
): Promise<boolean> {
 const batch=target.batch;
 if(!batch || terminalBatchStatuses.has(batch.status)) return false;
 const previousStatus=batch.status;
 batch.status='checking';
 batch.error=null;
 onChange();
 try {
  const result=batch.id
   ? await request(`/api/control/v1/batches/${encodeURIComponent(batch.id)}`)
   : await request('/api/control/v1/batches',batchRequestOptions(batch));
  return applyBatchResult(target,batch,result as BatchResponse,onChange);
 } catch(error) {
  if(target.batch!==batch) return false;
  batch.status='outcome_unknown';
  batch.error=errorMessage(error);
  if(previousStatus!=='outcome_unknown')notifyTarget(target,{key:`batch:${batch.id||batch.idempotencyKey}:unknown`,message:`The ${batch.action} batch result is unknown. Check its status before starting another batch.`,tone:'warning',action:{label:'Check status',action:'batch:check',icon:'refresh'}});
  onChange();
  return false;
 }
}
export function gattReadinessLabel(readiness: GattReadiness | null | undefined): string {
 switch (readiness?.status) {
  case 'untested': return 'No command sent yet';
  case 'queued': return 'Command queued';
  case 'connecting': return 'Command in progress';
  case 'deferred': return 'Paused by Bluetooth cooldown';
  case 'connection_failed': return 'Could not connect; no write sent';
  case 'write_acknowledged': return 'GATT write acknowledged';
  case 'write_failed': return 'Write failed; delivery uncertain';
  case 'outcome_unknown': return 'Write outcome unknown';
  case 'expired': return 'Expired before attempt';
  case 'cancelled': return 'Cancelled before write';
  default: return 'No command result available';
 }
}
export function gattTransportEvidenceLabel(evidence: string | undefined): string {
 const labels: Record<string, string> = {
  untested:'No transport evidence yet',
  command_pending:'A command is in progress',
  recently_acknowledged:'A GATT write was recently acknowledged',
  acknowledgement_aged:'The last GATT acknowledgement is old',
  prewrite_failure:'The latest command failed before a write',
  write_failed:'The latest write failed; delivery may be uncertain',
  outcome_unknown:'The latest write outcome is unknown',
  no_recent_write:'No recent write evidence',
  cooldown:'Movement command cooldown is active',
 };
 return (evidence ? labels[evidence] : undefined) || 'Transport evidence unavailable';
}
export function motionFeedbackLabel(feedback: MotionFeedback | null | undefined): string {
 const labels: Record<string, string> = {
  awaiting_progress:'Waiting for position progress',
  progress_observed:'Position progress observed',
  stalled:'No recent position progress',
  target_reached:'Motor reports the target position',
  unavailable:'Position feedback unavailable',
 };
 const status = feedback?.status;
 return (status ? labels[status] : undefined) || 'No movement feedback';
}
function dateTimeLabel(value: string | null | undefined, fallback: string): string {
 if (!value) return fallback;
 const date = new Date(value);
 return Number.isNaN(date.getTime()) ? 'Time unavailable' : date.toLocaleString();
}
function cardCommandMessage(command: CommandRecord | undefined): [string, VisualTone] {
 if (!command) return ['', ''];
 const contextual = !terminal.has(command.status)
  || ['outcome_unknown','write_failed','deferred'].includes(command.status);
 return contextual ? commandMessage(command) : ['', ''];
}
function cardAvailabilityDetail(device: ManagedDevice): string {
 return device.availability === 'ready'
  ? ''
  : 'Controls are unavailable until this blind is uniquely observed again.';
}
function cardCooldownDetail(cooldownUntil: string | null | undefined, now: number): string {
 if (!cooldownUntil || !(new Date(cooldownUntil).getTime() > now)) return '';
 return `Repeated Bluetooth connection failures. Open and Close are paused until ${dateTimeLabel(cooldownUntil,'the cooldown ends')}; Stop remains available.`;
}
function cardCalibrationDetail(device: ManagedDevice): string {
 return device.calibration_mode_active
  ? 'Endpoint calibration is active. Normal movement is paused until calibration ends.'
  : '';
}
export function cardMessageMarkup(
 panelMode: boolean,
 message: string,
 tone: VisualTone,
 availabilityDetail: string,
 cooldownDetail: string,
 calibrationDetail: string,
): string {
 if(!panelMode)return regularCardMessageMarkup(message,tone,availabilityDetail,cooldownDetail,calibrationDetail);
 const details=[message,availabilityDetail,cooldownDetail,calibrationDetail].filter(Boolean);
 if(!details.length)return '';
 return renderPanelNotice({
  label:panelCardNoticeLabel(message,availabilityDetail,cooldownDetail,calibrationDetail),
  details,
  tone:panelCardNoticeTone(tone,availabilityDetail,cooldownDetail,calibrationDetail),
 });
}
function panelCardNoticeLabel(message: string, availability: string, cooldown: string, calibration: string): string {
 if(cooldown)return 'Movement paused';
 if(calibration)return 'Calibration active';
 if(availability)return 'Blind unavailable';
 return message?'Command update':'';
}
function panelCardNoticeTone(tone: VisualTone, availability: string, cooldown: string, calibration: string): VisualTone {
 if(tone)return tone;
 return availability||cooldown||calibration?'warn':'good';
}
function regularCardMessageMarkup(
 message: string,
 tone: VisualTone,
 availability: string,
 cooldown: string,
 calibration: string,
): string {
 return [
  message?`<p class="message ${tone}" role="status">${esc(message)}</p>`:'',
  availability?`<p class="message warn">${esc(availability)}</p>`:'',
  cooldown?`<p class="message warn">${esc(cooldown)}</p>`:'',
  calibration?`<p class="message warn">${esc(calibration)}</p>`:'',
 ].join('');
}
export function renderManagedDeviceCard(
 device: ManagedDevice,
 dashboard: DashboardState,
 panelMode: boolean,
 now = Date.now(),
 showRoom = false,
): string {
 const e=device.enrollment, o=observationFor(device,dashboard.observations), [label,tone]=availability(device.availability), p=device.position?.current_closed_percent;
 const blindLabel=e.label ?? 'Blind';
 const cooldownUntil=device.gatt_readiness?.cooldown_until;
 const cooldownDetail=cardCooldownDetail(cooldownUntil,now);
 const [message,msgTone]=cardCommandMessage(dashboard.commands[e.id]);
 const targetPercent=device.position?.target_closed_percent ?? p ?? 100;
 const lines=typeof p === 'number' && Number.isInteger(p) ? 11 : 0;
 const disabled=!!dashboard.serviceError||globalThis.navigator?.onLine===false||device.availability!=='ready';
 const availabilityDetail=cardAvailabilityDetail(device);
 const calibrationDetail=cardCalibrationDetail(device);
 const controlsMarkup=renderBlindActions({
  deviceId:e.id, blindLabel,
  panelMode,
  disabled, busy:dashboard.busy.has(e.id),
  movementDisabled:!!cooldownDetail||device.calibration_mode_active===true||blindBatchIsLocked(dashboard.batch),
  supportsPosition:device.capabilities?.set_position === true,
  positionClosedPercent:targetPercent,
  currentClosedPercent:p,
  currentPositionFresh:device.position_freshness==='fresh',
 });
 let messageMarkup=cardMessageMarkup(panelMode,message,msgTone,availabilityDetail,cooldownDetail,calibrationDetail);
 const detailsButton=renderButton({label:'Details ›',accessibleLabel:`View details for ${blindLabel}`,action:`details:${e.id}`,variant:'details-link-button'});
 const footerMarkup=`${batteryDisplay(o?.battery,now)}${detailsButton}`;
  const command=dashboard.commands[e.id];
  return renderBlindCard({
  id:e.id, label:blindLabel, isSimulated:e.is_simulated, room:e.room, lastSeen:device.availability==='ready'?'':relativeAt(device.last_seen_at,now),
  availabilityLabel:label, availabilityTone:tone,
  availabilityStatus:device.availability,
  positionPercent:p, movementStatus:commandAwareMovementStatus(o?.movement_status ?? 'unavailable',command), blindLines:lines,
  controlsMarkup, messageMarkup, footerMarkup, panelMode,
  supportsPosition:device.capabilities?.set_position===true,
  showPositionCue:!positionCueDismissed&&dashboard.managed.find(item=>item.capabilities?.set_position===true)?.enrollment.id===e.id,
  showRoom:showRoom&&!panelMode,
 });
}
function card(device: ManagedDevice, showRoom = false): string {
 return renderManagedDeviceCard(device,state,document.documentElement.dataset.displayMode==='panel',Date.now(),showRoom);
}
function roomGroupMarkup(room: string, devices: ManagedDevice[]): string {
 const encoded=encodeURIComponent(room);
 const unassigned=room==='Unassigned';
 const actionsMarkup=unassigned?'':`${renderButton({label:'Rename room',action:`room-rename:${encoded}`,variant:'secondary'})}${renderDestructiveButton({label:'Remove room',action:`room-remove:${encoded}`})}`;
 let managementMarkup='';
 if(editingRoomName===room){
  managementMarkup=`<form class="room-group__management" data-room-rename="${esc(encoded)}" novalidate><label class="field" for="room-name-${esc(encoded)}">Room name<input id="room-name-${esc(encoded)}" name="name" maxlength="40" required value="${esc(room)}" data-room-name-input aria-invalid="false" aria-describedby="room-name-${esc(encoded)}-error"><span id="room-name-${esc(encoded)}-error" class="field-error" data-room-name-error hidden role="alert"></span></label><div class="room-actions">${renderButton({label:'Cancel',action:`room-edit-cancel:${encoded}`,variant:'secondary'})}${renderButton({label:'Save room',type:'submit'})}</div></form>`;
 }else if(confirmingRoomRemoval===room){
  managementMarkup=`<div class="room-group__management">${renderConfirmationPanel({title:`Remove ${room}?`,message:`${devices.length} ${devices.length===1?'blind will':'blinds will'} move to Unassigned. Their history will stay with each blind.`,confirmLabel:'Remove room',confirmAction:`room-remove-confirm:${encoded}`,cancelAction:`room-remove-cancel:${encoded}`})}</div>`;
 }
 return renderRoomGroup({
  room,
  actionsMarkup,
  managementMarkup,
 });
}
function activityPanel() {
 const entries = state.activity || [];
 const list = entries.length
  ? `<ol class="activity-list">${entries.map(entry=>{
   const occurredAt=entry.completed_at||entry.occurred_at;
   return `<li class="activity-item"><span class="activity-copy">${esc(activityMessage(entry))}${entry.detail&&entry.kind!=='simulator_settings_changed'&&entry.kind!=='simulator_reset'?`<span class="subtle">${esc(entry.detail)}</span>`:''}</span>${renderRelativeTime(occurredAt,Date.now())}</li>`;
  }).join('')}</ol>`
  : '<div class="panel activity-empty"><p>Changes you make will show up here.</p></div>';
 const expanded=activityCollapsedByUser??document.documentElement.dataset.displayMode!=='mobile';
 return renderActivitySection({count:entries.length,expanded,listMarkup:list});
}
function home() {
 const panelMode = document.documentElement.dataset.displayMode === 'panel';
 const empty = panelMode
  ? renderEmptyState({title:'No blinds yet',description:'Add a nearby blind to see its status and controls here.'})
  : renderEmptyState({title:'Let’s bring your blinds in.',description:'When a blind is nearby and discovered, add it here to see its status and control it.',icon:'home',actionMarkup:renderIconButton({label:'Add a blind',action:'add',icon:'add'})});
 const batchControls=renderBatchControls({
  blindCount:state.managed.length,
  eligibleCount:batchEligibleCount(state.managed),
  disabled:!!state.serviceError||globalThis.navigator?.onLine===false||state.busy.size>0,
  busy:blindBatchIsLocked(state.batch),
  panelMode,
  batch:state.batch,
 });
 const loadingError=state.serviceError&&!state.managed.length?`<div class="panel error-state" role="alert"><span class="error-state__icon" role="img" aria-label="Connection problem">${renderIcon('warning')}</span><h3>Somaris is disconnected</h3><p>${esc(globalThis.navigator?.onLine===false?'You are offline. Reconnect and try again.':'Could not load your blinds. Check the connection and try again.')}</p>${renderIconButton({label:'Retry loading blinds',action:'refresh',icon:'refresh',variant:'secondary'})}</div>`:'';
 const roomGroups=!panelMode&&state.managed.length
  ? [...roomNamesFromDevices(state.managed).map(room=>({
     room,
     devices:state.managed.filter(device=>device.enrollment.room?.toLowerCase()===room.toLowerCase()),
    })),{
     room:'Unassigned',
     devices:state.managed.filter(device=>!device.enrollment.room),
    }].filter(group=>group.devices.length)
  : [];
 const roomGroupsMarkup=roomGroups.map(group=>roomGroupMarkup(group.room,group.devices)).join('');
 const roomCardsMarkup=roomGroups.flatMap(group=>group.devices).map(device=>card(device,true)).join('');
 const groupsMarkup=roomGroups.length?renderRoomCollection({groupsMarkup:roomGroupsMarkup,cardsMarkup:roomCardsMarkup}):'';
 const listMarkup=renderDeviceList({
  count:state.managed.length, panelMode,
  cardsMarkup:state.managed.map(device=>card(device)).join(''), emptyMarkup:loadingError||empty, toolbarMarkup:batchControls,
  groupsMarkup,
 });
 const serviceMarkup=renderServiceSummary(scannerSummary());
 const demoHint=state.demoMode&&!hintDismissed&&!panelMode?`<aside class="demo-hint"><span>${renderIcon('target')} Drag a blind graphic to set its position, or use Open, Stop, and Close for quick moves.</span>${renderDismissButton({label:'Dismiss demo hint',action:'demo-hint-dismiss',variant:'demo-hint-dismiss'})}</aside>`:'';
 const supportingSections=renderHomeSupportingSections({
  panelMode,
  activityMarkup:panelMode?'':activityPanel(),
  serviceMarkup:panelMode?'':serviceMarkup,
 });
 return renderHomeContent({deviceListMarkup:listMarkup,demoHintMarkup:demoHint,supportingMarkup:supportingSections});
}
function scannerSummary(){ const s=state.diagnostics?.scanner; if(state.demoMode)return state.diagnostics?.simulator_ready?'Everything is working':'Simulator unavailable'; if(!s) return 'Service status unavailable'; return s.adapter_available ? (s.scan_active?'Bluetooth adapter available · scanning':'Bluetooth adapter available') : 'Bluetooth adapter unavailable · retrying'; }
function back(){ return renderBackButton(); }
function addPage(){
 const available=state.observations.filter(o=>!state.managed.some(m=>m.enrollment.logical_id===o.logical_id && !!m.enrollment.is_simulated===!!o.is_simulated));
 const intro=state.demoMode?'This demo lists simulated blinds only; Bluetooth discovery is unavailable here.':'Choose a recently discovered blind to make it available for control.';
 const nearby=available.length?`<div class="section-heading"><h2>Nearby devices</h2><span class="count">${available.length} found</span></div><div class="stack">${available.map(o=>{
  const deviceName=o.advertised_name||o.device_name||'Unnamed blind';
  const fieldId=`display-name-${encodeURIComponent(o.id)}`;
  const errorId=`${fieldId}-error`;
  return `<form class="panel device-row" data-enroll="${esc(o.id)}" novalidate><div><h3>${esc(deviceName)}</h3><p class="subtle">${o.is_simulated?'Simulator · ':''}${esc(relative(o.last_seen_at))} · ${esc(o.freshness)}</p><label class="field" for="${esc(fieldId)}">Display name<input id="${esc(fieldId)}" name="label" maxlength="64" required value="${esc(o.advertised_name||o.device_name||'Blind')}" aria-describedby="${esc(errorId)}" aria-invalid="false" data-display-name><span class="field-error" id="${esc(errorId)}" hidden role="alert"></span></label>${renderRoomPicker({id:`room-choice-${encodeURIComponent(o.id)}`,rooms:roomNamesFromDevices(state.managed)})}</div>${renderButton({label:'Add blind',accessibleLabel:`Add ${deviceName}`,title:`Add ${deviceName}`,type:'submit',icon:'add',iconOnly:true,variant:'secondary',disabled:o.freshness!=='fresh'})}</form>`;
 }).join('')}</div>`:state.demoMode?renderAddDeviceEmptyState({demoMode:true,adapterAvailable:false}):`<div class="section-heading"><h2>Nearby devices</h2><span class="count">0 found</span></div>${renderAddDeviceEmptyState({demoMode:false,adapterAvailable:state.diagnostics?.scanner?.adapter_available===true})}`;
 return `${back()}<h1 class="page-title">Add a blind</h1><p class="page-intro">${intro}</p>${nearby}`;
}
export interface DeviceDetailField {label: string; value?: string | number | null; contentMarkup?: string; advanced?: boolean}

export function deviceDetailFields(
 device: ManagedDevice,
 observation: ObservationIdentity | null,
 now = Date.now(),
): DeviceDetailField[] {
 const fields: DeviceDetailField[] = [
  {label:'Availability',value:availability(device.availability)[0]},
  {label:'Position',contentMarkup:renderBlindStatus({positionPercent:device.position?.current_closed_percent,movementStatus:observation?.movement_status ?? 'unavailable',compact:true})},
  {label:'Last heard from',value:device.last_seen_at?new Date(device.last_seen_at).toLocaleString():'Never'},
  {label:'Advertisement age',value:typeof device.advertisement_age_ms === 'number' && Number.isFinite(device.advertisement_age_ms)?`${Math.floor(device.advertisement_age_ms/1000)} seconds`:'Unavailable',advanced:true},
  {label:'Reported target',value:device.position?`${device.position.target_closed_percent}% closed`:'Unavailable',advanced:true},
  {label:'Movement feedback',value:motionFeedbackLabel(device.motion_feedback),advanced:true},
 ];
 const travelRate=device.motion_feedback?.estimated_travel_rate_percent_per_second;
 if(typeof travelRate === 'number' && Number.isFinite(travelRate)) {
  fields.push({label:'Learned travel rate',value:`${travelRate.toFixed(2)}% per second`,advanced:true});
 }
 fields.push(
  {label:'Signal strength',value:observation?.rssi_dbm!=null?`${observation.rssi_dbm} dBm`:'Unavailable'},
  {label:'Battery',value:observation?.battery?.percent_estimate!=null?`~${observation.battery.percent_estimate}%${observation.battery.observed_at?` · Last reading ${relativeTimeLabel(observation.battery.observed_at,now)}`:''}`:'Unavailable'},
  {label:'Battery voltage',value:observation?.battery?.voltage_mv!=null?`${observation.battery.voltage_mv} mV${observation.battery.observed_at?` · Last reading ${relativeTimeLabel(observation.battery.observed_at,now)}`:''}`:'Unavailable',advanced:true},
  {label:'Source',value:device.enrollment.is_simulated?'Simulator':'Bluetooth'},
 );
 if(device.gatt_readiness)fields.push(...gattDetailFields(device.gatt_readiness,now));
 return fields;
}

function gattDetailFields(readiness: GattReadiness, now: number): DeviceDetailField[] {
 const fields: DeviceDetailField[] = [
  {label:'Last GATT command',value:gattReadinessLabel(readiness),advanced:true},
  {label:'Transport evidence',value:gattTransportEvidenceLabel(readiness.transport_evidence),advanced:true},
  {label:'Last command attempt',value:dateTimeLabel(readiness.last_attempt_at,'Not attempted yet'),advanced:true},
  {label:'Last acknowledged GATT write',value:dateTimeLabel(readiness.last_write_acknowledged_at,'None recorded'),advanced:true},
 ];
 if(readiness.cooldown_until && new Date(readiness.cooldown_until).getTime()>now) {
  fields.push({label:'Movement cooldown until',value:dateTimeLabel(readiness.cooldown_until,'None'),advanced:true});
 }
 fields.push({label:'Latest failure',value:readiness.error_code?`${readiness.error_code} · ${readiness.failure_stage||'unknown phase'}`:'None recorded',advanced:true});
 return fields;
}

function enrollmentConfirmation(id: string, device: ManagedDevice): string {
 const encodedId=encodeURIComponent(id);
 if(state.confirmingRemovalId!==id) {
  return renderDestructiveButton({label:'Remove blind', action:`remove:${encodedId}`});
 }
 return renderConfirmationPanel({
  title:`Remove ${device.enrollment.label ?? 'Blind'}?`,
  message:'This removes the enrollment from Somaris. It does not reset or change the physical blind, and the blind can be added again when it is nearby. Removal is blocked while a command is queued or running.',
  confirmLabel:'Remove blind',
  confirmAction:`remove-confirm:${encodedId}`,
  cancelAction:`remove-cancel:${encodedId}`,
  disabled:state.removingEnrollmentId===id,
 });
}

function calibrationSection(id: string, device: ManagedDevice, busy: boolean): string {
 if(!device.capabilities?.calibrate_endpoints)return '';
 const warning=device.calibration_mode_active
  ? '<p class="message warn" role="status">Calibration mode is active. Watch the blind while nudging it, then mark each endpoint.</p>'
  : '';
 return `<section class="panel device-settings"><h2>Endpoint calibration</h2><p>Calibration changes the blind’s stored open and closed limits. Stay with the blind while adjusting it. Use the small nudges to reach each endpoint, mark both limits, then end calibration.</p>${warning}${renderCalibrationControls({deviceId:id,active:device.calibration_mode_active===true,busy})}</section>`;
}

function roomAssignmentSection(device: ManagedDevice): string {
 const id=device.enrollment.id;
 const encodedId=encodeURIComponent(id);
 return `<section class="panel room-assignment"><h2>Room</h2><p>Choose where this blind belongs. Choose Unassigned to remove it from a room.</p><form data-room-assignment="${esc(encodedId)}" novalidate>${renderRoomPicker({id:`device-room-${encodedId}`,rooms:roomNamesFromDevices(state.managed),selectedRoom:device.enrollment.room||null})}<div class="room-assignment__actions">${renderButton({label:'Save room',type:'submit'})}</div></form></section>`;
}

function detailPage(id: string): string {
 const device=state.managed.find(item=>item.enrollment.id===id);
 if(!device)return `${back()}<div class="notice">This blind is no longer enrolled.</div>`;
 const observation=observationFor(device,state.observations);
 const busy=state.busy.has(id);
 const detailFields=deviceDetailFields(device,observation);
 const fields=renderInfoGrid(detailFields.filter(field=>!field.advanced));
 const advancedFields=detailFields.filter(field=>field.advanced);
 const advanced=advancedFields.length?renderDisclosure({label:'Advanced device diagnostics',variant:'advanced-disclosure',contentMarkup:`<h2>Advanced diagnostics</h2>${renderInfoGrid(advancedFields)}`}):'';
 const motorSettings=device.capabilities?.set_motor_speed
  ? renderMotorSpeedControl({deviceId:id,busy,calibrationActive:device.calibration_mode_active===true})
  : '';
 const calibration=calibrationSection(id,device,busy);
 const enrollment=enrollmentConfirmation(id,device);
 return `${back()}<h1 class="page-title">${esc(device.enrollment.label ?? 'Blind')}</h1><p class="page-intro">The latest observations reported by your blind.</p>${roomAssignmentSection(device)}${fields}<p class="message">Position is observed telemetry and may not reflect the current physical position.</p>${advanced}${motorSettings}${calibration}<section class="panel enrollment-actions"><h2>Manage enrollment</h2><p>Remove this blind from your Somaris dashboard. If it is still nearby, it will remain discoverable and can be added again.</p>${enrollment}</section>`;
}
function simulatorControls(): string {
 if(!state.simulators?.length)return '';
 type SimulatorOption = readonly [string, string];
 const options=(values: readonly SimulatorOption[],current: string)=>values.map(([value,label])=>`<option value="${value}" ${value===current?'selected':''}>${label}</option>`).join('');
 const scenarios=[['normal','Normal'],['pre_write_failure','Connection failure'],['outcome_unknown','Unknown outcome'],['gatt_write_error','Write error'],['telemetry_paused','Telemetry paused']] as const;
 const profiles=[['standard','Standard'],['hallway_observed','Hallway observed'],['hallway_slow_link','Hallway slow link']] as const;
 const batteries=[['unsupported','Unsupported'],['unavailable','Unavailable'],['observed_sample','Observed sample']] as const;
 const instances=state.simulators.map(simulator=>{
  const movementStatus=movementStatusFromPositions(simulator.current_closed_percent,simulator.target_closed_percent);
  const position=renderBlindStatus({positionPercent:simulator.current_closed_percent,movementStatus,compact:true});
  return `<div class="simulator-controls"><h3>${esc(simulator.advertised_name)}</h3><div class="simulator-position">${position}<span class="subtle">Simulator ID ${esc(simulator.id)}</span></div><form data-simulator="${esc(simulator.id)}"><label class="field">Scenario<select name="scenario">${options(scenarios,simulator.scenario ?? '')}</select></label><label class="field">Wireless profile<select name="wireless_profile">${options(profiles,simulator.wireless_profile ?? '')}</select></label><label class="field">Battery state<select name="battery_state">${options(batteries,simulator.battery_state ?? '')}</select></label><button class="button">Apply settings</button></form><button class="button secondary" style="margin-top:12px" data-action="sim-reset:${esc(simulator.id)}">Reset to closed</button></div>`;
 }).join('');
 return `<section class="panel" style="margin-top:28px"><p class="eyebrow">Development</p><h2>Simulator controls</h2><p class="subtle">Settings and resets apply only to the selected simulator.</p>${instances}</section>`;
}
function serviceStatusFields(): {fields:{label:string;value:string|number}[];notUsed:{label:string;value:string|number}[]} {
 const scanner=state.diagnostics?.scanner;
 if(state.demoMode){
  return {
   fields:[{label:'Simulated blinds',value:state.simulators?.length??state.managed.length}],
   notUsed:[
    {label:'Bluetooth adapter',value:'Not used here'},
    {label:'Scanner',value:'Not used here'},
    {label:'Blind signals found',value:'Not collected'},
    {label:'Scanner errors',value:'Not applicable'},
    {label:'Telemetry history',value:state.diagnostics?.history_enabled?'Enabled':'Not used here'},
   ],
  };
 }
 return {fields:[
  {label:'Bluetooth adapter',value:scanner?.adapter_available?'Available':'Unavailable'},
  {label:'Scanner',value:scanner?.status||'Unavailable'},
  {label:'Blind signals found',value:scanner?.soma_advertisements_seen??'—'},
  {label:'Last scan issue',value:scanner?.last_error_code||'None reported'},
  {label:'Telemetry history',value:state.diagnostics?.history_enabled?'Enabled':'Not enabled'},
  {label:'Simulators',value:state.simulators?.length??'Unavailable'},
 ],notUsed:[]};
}
function serviceStatusSummary():string {
 if(state.serviceError)return 'The service connection needs attention';
 if(state.demoMode)return state.diagnostics?.simulator_ready?'Everything is working':'The simulator is unavailable';
 return state.diagnostics?.scanner?.adapter_available?'Everything is working':'Bluetooth scanning is unavailable';
}
function settingsPage(): string {
 const data=serviceStatusFields();
 const notUsedMarkup=state.demoMode?renderInfoGrid(data.notUsed):'';
 return renderSettingsPage({themePreference,serviceSummary:serviceStatusSummary(),serviceFields:data.fields,notUsedMarkup,simulatorMarkup:simulatorControlsForMode()});
}
function simulatorControlsForMode(): string {
 return state.demoMode?'':simulatorControls();
}
function page(): string {return location.hash.slice(1)||'home'}
function pageContent(route: string): string {
 if(route==='add')return addPage();
 if(route==='settings'||route==='service')return settingsPage();
 if(route.startsWith('details/'))return detailPage(decodeURIComponent(route.slice(8)));
 return home();
}
function topbarStatus(): {serviceLabel: string; serviceShortLabel:string;serviceTone: VisualTone} {
 const diagnostics=state.diagnostics;
 if(globalThis.navigator?.onLine===false)return {serviceLabel:'Disconnected',serviceShortLabel:'Offline',serviceTone:'bad'};
 if(state.serviceError)return {serviceLabel:'Disconnected',serviceShortLabel:'Offline',serviceTone:'bad'};
 if(!diagnostics)return {serviceLabel:'Connecting',serviceShortLabel:'Loading',serviceTone:''};
 if(diagnostics.demo_mode && diagnostics.simulator_ready) {
  return {serviceLabel:'Connected',serviceShortLabel:'Ready',serviceTone:'good'};
 }
 if(diagnostics.scanner?.adapter_available)return {serviceLabel:'Connected',serviceShortLabel:'Ready',serviceTone:'good'};
 return {serviceLabel:'Service degraded',serviceShortLabel:'Degraded',serviceTone:''};
}
function updateTopbar(): void {
 const topActions=document.querySelector<HTMLElement>('#top-actions');
 if(!topActions)return;
 const markup=renderTopbarActions({
  blindCount:state.managed.length,
  panelMode:document.documentElement.dataset.displayMode==='panel',
  demoMode:state.demoMode,
  displayPreference,
  themePreference,
  route:page(),
  ...topbarStatus(),
 });
 if(topActions.innerHTML===markup)return;
 const activeElement=document.activeElement;
 const active=activeElement instanceof HTMLElement && topActions.contains(activeElement) ? activeElement : null;
 const focusId=active?.id || '';
 const focusLabel=active?.getAttribute('aria-label') || '';
 topActions.innerHTML=markup;
 const replacement=focusId ? document.getElementById(focusId)
  : focusLabel ? [...topActions.querySelectorAll<HTMLElement>('[aria-label]')].find(el=>el.getAttribute('aria-label')===focusLabel)
   : null;
 replacement?.focus({preventScroll:true});
}
function restoreAppFocus(appRoot: HTMLElement, focusedAction: string | null): void {
 const actionTargets=[...appRoot.querySelectorAll<HTMLElement>('[data-action]')];
 const preferredAction=resolveActionFocus({
  focusedAction,
  pendingConfirmationId:state.pendingRemovalFocusId,
  confirmingRemovalId:state.confirmingRemovalId,
  availableActions:actionTargets.map(el=>el.dataset.action ?? ''),
 });
 state.pendingRemovalFocusId=null;
 if(!preferredAction)return;
 actionTargets.find(element=>element.dataset.action===preferredAction)?.focus({preventScroll:true});
}
function render(): void {
 if(!app)return;
 const scrollY=window.scrollY;
 const activeElement=document.activeElement;
 const focusedPositionGraphic=activeElement instanceof HTMLElement?activeElement.dataset.positionGraphic:null;
 const focusedPositionInput=activeElement instanceof HTMLElement?activeElement.dataset.positionInput:null;
 const focusedInputId=activeElement instanceof HTMLElement&&activeElement.matches('input,select,textarea')?activeElement.id:'';
 const focusedAction=activeElement instanceof HTMLElement && app.contains(activeElement)
  ? activeElement.dataset.action ?? null
  : null;
 const content=pageContent(page());
 document.documentElement.dataset.theme=themePreference;
 app.removeAttribute('aria-busy');
 app.removeAttribute('aria-label');
 app.innerHTML=`${state.demoMode?renderDemoBanner():''}${content}${document.documentElement.dataset.displayMode==='panel'?'':renderMobileTabs(page())}`;
 for(const popover of app.querySelectorAll<HTMLElement>('[popover]'))popover.addEventListener('toggle',()=>{
  const trigger=app?.querySelector<HTMLButtonElement>(`[aria-controls="${CSS.escape(popover.id)}"]`);
  trigger?.setAttribute('aria-expanded',String(popover.matches(':popover-open')));
 });
 updateTopbar();
 restoreAppFocus(app,focusedAction);
 if(focusedPositionGraphic)app.querySelector<HTMLElement>(`[data-position-graphic="${CSS.escape(focusedPositionGraphic)}"]`)?.focus({preventScroll:true});
 else if(focusedPositionInput)app.querySelector<HTMLInputElement>(`[data-position-input="${CSS.escape(focusedPositionInput)}"]`)?.focus({preventScroll:true});
 else if(focusedInputId)document.getElementById(focusedInputId)?.focus({preventScroll:true});
 window.scrollTo({top:scrollY,behavior:'instant'});
}
export async function refreshService(target: RefreshTarget, request: ApiRequest): Promise<void> {
  try {
    const [managed, observations, diagnostics, simulators, activity] = await Promise.all([
      request('/api/control/v1/devices'), request('/api/v1/devices'),
      request('/api/v1/diagnostics'), request('/api/dev/simulators').catch(() => null),
      request('/api/control/v1/audit-log'),
    ]);
    Object.assign(target, {
      managed:managed as ManagedDevice[],
      observations:observations as Observation[],
      diagnostics:diagnostics as Diagnostics,
      demoMode:(diagnostics as Diagnostics).demo_mode === true,
      simulators:simulators as Simulator[] | null,
      activity:activity as ActivityEntry[],
    });
    serviceRecovered(target);
    const released = releaseReconciledCommandLocks(target);
    if (released) notifyTarget(target,{message:'A fresh position is available. Verify it before trying again.',tone:'warning'});
  } catch (error) {
    serviceFailure(target,error);
  }
}

export async function resetDemo(
 target: RefreshTarget,
 request: ApiRequest,
 refreshData: () => Promise<unknown>,
 confirmReset: () => boolean = () => true,
): Promise<boolean> {
 if(!confirmReset())return false;
 try{
  await request('/api/demo/reset',{method:'POST'});
  if(target.commands)target.commands={};
  if(target.batch!==undefined)target.batch=null;
  target.busy?.clear();
  target.reconciliationRequired?.clear();
  if(target.tokens)target.tokens={};
  await refreshData();
  notifyTarget(target,{message:'Your demo was reset to the original household.',tone:'success'});
  return true;
 }catch(error){
  notifyTarget(target,{message:`Could not reset the demo: ${errorMessage(error)}`,tone:'error'});
  return false;
 }
}

function hasPostCommandFreshPosition(command: CommandRecord | undefined, device: ManagedDevice | null | undefined): boolean {
  const acceptedAt = Date.parse(command?.accepted_at || '');
  const observedAt = Date.parse(device?.position_observed_at || '');
  return device?.availability === 'ready'
    && device.position_freshness === 'fresh'
    && Number.isFinite(acceptedAt)
    && Number.isFinite(observedAt)
    && observedAt >= acceptedAt;
}

function releaseReconciledCommandLocks(target: RefreshTarget): boolean {
  const required = target.reconciliationRequired;
  if (!(required instanceof Set)) return false;
  let released = false;
  for (const id of required) {
    const command = target.commands?.[id];
    if (command && ['queued', 'connecting'].includes(command.status)) continue;
    const device = target.managed?.find(item => item.enrollment?.id === id);
    if (!hasPostCommandFreshPosition(command, device)) continue;
    required.delete(id);
    target.busy?.delete(id);
    released = true;
  }
  return released;
}

export async function refreshCommandTelemetry(target: RefreshTarget, request: ApiRequest, managedDeviceId: string): Promise<{positionFresh: boolean; errors: string[]}> {
  const [managedResult, observationsResult] = await Promise.allSettled([
    Promise.resolve().then(() => request('/api/control/v1/devices')),
    Promise.resolve().then(() => request('/api/v1/devices')),
  ]);
  const failures = [];
  if (managedResult.status === 'fulfilled') target.managed = managedResult.value as ManagedDevice[];
  else failures.push(`Managed devices: ${errorMessage(managedResult.reason)}`);
  if (observationsResult.status === 'fulfilled') target.observations = observationsResult.value as Observation[];
  else failures.push(`Observations: ${errorMessage(observationsResult.reason)}`);
  const device = managedResult.status === 'fulfilled'
    ? (managedResult.value as ManagedDevice[]).find(item => item.enrollment?.id === managedDeviceId)
    : null;
  return {
    positionFresh: hasPostCommandFreshPosition(target.commands?.[managedDeviceId], device),
    errors: failures,
  };
}

export async function handleCommandPollFailure(
 target: RefreshTarget & {commands: Record<string, CommandRecord>; tokens: Record<string, symbol>},
 id: string,
 token: symbol,
 error: unknown,
 refreshData: () => Promise<{positionFresh: boolean; errors?: string[]} | void>,
 onChange: () => void = () => {},
): Promise<boolean> {
  if (target.tokens?.[id] !== token) return false;
  const reason = errorMessage(error).replace(/[.!?]+$/, '');
  target.commands[id] = {...target.commands[id],status:'outcome_unknown'};
  target.reconciliationRequired ??= new Set();
  target.reconciliationRequired.add(id);
  onChange();
  const refreshResult = await refreshData();
  if (target.tokens?.[id] !== token) return false;
  const errors = refreshResult && refreshResult.errors?.length ? ` Some telemetry could not be refreshed: ${refreshResult.errors.join('; ')}.` : '';
  if (refreshResult && refreshResult.positionFresh === true) {
    target.reconciliationRequired.delete(id);
    target.busy?.delete(id);
    notifyTarget(target,{key:`command-status:${id}`,message:`Could not check command status: ${reason}. Check the latest reported position before trying again.${errors}`,tone:'warning',action:{label:'Refresh position',action:'refresh',icon:'refresh'}});
  } else {
    notifyTarget(target,{key:`command-status:${id}`,message:`Could not check command status: ${reason}. A fresh managed-device position is unavailable; movement controls remain locked. Refresh and verify the position before trying again.${errors}`,tone:'warning',action:{label:'Refresh position',action:'refresh',icon:'refresh'}});
  }
  onChange();
  return true;
}

export async function enrollBlind(target: RefreshTarget, request: ApiRequest, refreshData: () => Promise<unknown>, observedDeviceId: string, label: string, room?: string | null): Promise<boolean> {
  try {
    const body: Record<string, unknown>={observed_device_id:observedDeviceId,label};
    if(room!==undefined)body.room=room;
    await request('/api/control/v1/devices', {
      method:'POST', headers:{'Content-Type':'application/json'},
      body:JSON.stringify(body),
    });
  } catch (error) {
    try { await refreshData(); } catch(refreshError) { serviceFailure(target,refreshError); }
    const failure=apiError(error);
    const message = failure.code === 'simulator_limit_reached'
      ? 'Could not add simulator: You can add up to two simulators. Remove one from your managed blinds before adding another.'
      : `Could not add blind: ${failure.message.replace(/[.!?]+$/, "")}. Refresh and check the list before retrying.`;
    notifyTarget(target,{message,tone:'error'});
    return false;
  }
  try { await refreshData(); } catch(refreshError) { serviceFailure(target,refreshError); }
  notifyTarget(target,{message:`${label} added.`,tone:'success'});
  return true;
}

async function executeRoomMutation(
 target:RefreshTarget,
 request:ApiRequest,
 refreshData:()=>Promise<unknown>,
 path:string,
 method:'PATCH'|'DELETE',
 body:Record<string,unknown>|undefined,
 successMessage:string,
):Promise<boolean>{
 try{
  await request(path,{method,headers:body?{'Content-Type':'application/json'}:undefined,body:body?JSON.stringify(body):undefined});
 }catch(error){
  try{await refreshData();}catch(refreshError){serviceFailure(target,refreshError);}
  notifyTarget(target,{message:`Room change failed: ${errorMessage(error)}`,tone:'error'});
  return false;
 }
 try{await refreshData();}catch(refreshError){serviceFailure(target,refreshError);}
 notifyTarget(target,{message:successMessage,tone:'success'});
 return true;
}

export async function updateBlindRoom(target:RefreshTarget,request:ApiRequest,refreshData:()=>Promise<unknown>,managedDeviceId:string,room:string|null):Promise<boolean>{
 if(room!==null){
  const validation=validateRoomName(room);
  if(validation){notifyTarget(target,{message:validation,tone:'error'});return false;}
 }
 return executeRoomMutation(target,request,refreshData,`/api/control/v1/devices/${encodeURIComponent(managedDeviceId)}`,'PATCH',{room},'Room assignment saved.');
}

export async function renameRoom(target:RefreshTarget,request:ApiRequest,refreshData:()=>Promise<unknown>,room:string,newName:string):Promise<boolean>{
 const validation=validateRoomName(newName);
 if(validation){notifyTarget(target,{message:validation,tone:'error'});return false;}
 return executeRoomMutation(target,request,refreshData,`/api/control/v1/rooms/${encodeURIComponent(room)}`,'PATCH',{name:newName.trim()},`Renamed ${room} to ${newName.trim()}.`);
}

export async function removeRoom(target:RefreshTarget,request:ApiRequest,refreshData:()=>Promise<unknown>,room:string):Promise<boolean>{
 return executeRoomMutation(target,request,refreshData,`/api/control/v1/rooms/${encodeURIComponent(room)}`,'DELETE',undefined,`Removed ${room}; its blinds are now Unassigned.`);
}

export async function removeEnrolledBlind(target: RefreshTarget, request: ApiRequest, refreshData: () => Promise<unknown>, managedDeviceId: string): Promise<boolean> {
  if (target.removingEnrollmentId === managedDeviceId) return false;
  target.removingEnrollmentId = managedDeviceId;

  try {
    await request(`/api/control/v1/devices/${encodeURIComponent(managedDeviceId)}`, {method:'DELETE'});
  } catch (error) {
    try {
      await refreshData();
    } catch (refreshError) {
      serviceFailure(target,refreshError);
    }
    const reason = errorMessage(error).replace(/[.!?]+$/, '');
    notifyTarget(target,{message:`Could not remove blind: ${reason}. Refresh the list and check the enrollment before retrying.`,tone:'error'});
    target.removingEnrollmentId = null;
    return false;
  }

  const removed=target.managed?.find(device=>device.enrollment.id===managedDeviceId);
  if (Array.isArray(target.managed)) {
    target.managed = target.managed.filter(device => device.enrollment.id !== managedDeviceId);
  }
  notifyTarget(target,{message:`${removed?.enrollment.label||'Blind'} removed.`,tone:'success'});
  if (target.commands) delete target.commands[managedDeviceId];
  target.busy?.delete(managedDeviceId);
  if (target.tokens) delete target.tokens[managedDeviceId];
  target.confirmingRemovalId = null;
  target.removingEnrollmentId = null;

  try {
    await refreshData();
  } catch (error) {
    serviceFailure(target,error);
  }
  return true;
}

async function refresh(quiet = false): Promise<void> {
 const active=document.activeElement;
 if(quiet&&(shouldPauseRefresh({dragging:positionInteraction,focusedTag:active?.tagName||'',confirmationOpen:!!state.confirmingRemovalId})||active instanceof HTMLElement&&active.matches('button,a,summary,[role="slider"]')))return;
 await refreshService(state,api);
 updatePositionFeedback();
 if(quiet)patchCards();
 else render();
}

function beginPositionInteraction(id: string, initial: number): void {
 dismissPositionCue();
 positionInteraction=true;
 positionInteractionId=id;
 positionStartValues.set(id,initial);
}

function previewPosition(id: string, value: number): void {
 const position=Math.max(0,Math.min(100,Math.round(value)));
 const positionClosedPercent=closedPercentFromOpenPercent(position);
 const output=app?.querySelector<HTMLOutputElement>(`[data-position-output="${CSS.escape(id)}"]`);
 if(output)output.value=`${position}%`;
 const graphic=app?.querySelector<HTMLElement>(`[data-position-graphic="${CSS.escape(id)}"]`);
 if(graphic){
  graphic.setAttribute('aria-valuenow',String(position));
  graphic.setAttribute('aria-valuetext',`${position}% open`);
  graphic.dataset.preview=String(position);
  graphic.querySelector<HTMLElement>('.blind-art__fill')?.style.setProperty('--blind-fill',`${positionClosedPercent}%`);
 }
 const range=app?.querySelector<HTMLInputElement>(`[data-position-input="${CSS.escape(id)}"]`);
 if(range&&document.activeElement!==range)range.value=String(position);
}

function commitPosition(id: string, value: number): void {
 const position=Math.max(0,Math.min(100,Math.round(value)));
 const previous=positionStartValues.get(id);
 positionStartValues.delete(id);
 positionInteraction=false;
 positionInteractionId=null;
 if(previous===undefined||previous===position)return;
 const last=lastPositionCommit.get(id);
 if(last?.value===position&&Date.now()-last.at<500)return;
 lastPositionCommit.set(id,{value:position,at:Date.now()});
 const device=state.managed.find(item=>item.enrollment.id===id);
 if(!device||device.capabilities?.set_position!==true||state.serviceError||device.availability!=='ready')return;
 void sendCommand(id,'set_position',closedPercentFromOpenPercent(position));
}

function cancelPositionInteraction(id: string): void {
 const previous=positionStartValues.get(id);
 positionStartValues.delete(id);
 positionInteraction=false;
 positionInteractionId=null;
 if(previous!==undefined)previewPosition(id,previous);
}

function graphicValueFromPointer(graphic: HTMLElement, clientY: number): number {
 const rect=graphic.getBoundingClientRect();
 return rect.height>0?Math.round((1-Math.max(0,Math.min(1,(clientY-rect.top)/rect.height)))*100):100;
}

function dismissPositionCue():void {
 if(positionCueDismissed)return;
 positionCueDismissed=true;
 try{globalThis.localStorage?.setItem('somaris-position-cue-dismissed','true');}catch{}
 for(const cue of app?.querySelectorAll<HTMLElement>('.position-cue')||[])cue.hidden=true;
}

function invalidateUndoFor(id: string): void {
 const feedback=positionFeedback.get(id);
 if(feedback){positionFeedback.delete(id);snackbarQueue.dismissByKey(feedback.key);}
 if(positionUndo?.id===id){positionUndo=null;const button=document.querySelector<HTMLButtonElement>(`[data-action^="undo-position:${CSS.escape(encodeURIComponent(id))}:"]`);if(button)button.disabled=true;}
 if(batchUndo?.previous.has(id)){
  batchUndo=null;
  const button=document.querySelector<HTMLButtonElement>('[data-action="undo-batch"]');
  if(button)button.disabled=true;
 }
}

async function undoBatchTargets(): Promise<void> {
 const snapshot=batchUndo;
 if(!snapshot||snapshot.expires<Date.now())return;
 batchUndo=null;
 for(const [id,position] of snapshot.previous){
  const device=state.managed.find(item=>item.enrollment.id===id);
  if(!device)continue;
  if(device.capabilities?.set_position===true)await sendCommand(id,'set_position',position);
  else if(position===0||position===100)await sendCommand(id,position===0?'open':'close');
 }
}

export function createServiceWorkerUpdateHandler(reload:()=>void): {
 requestActivation:(worker:Pick<ServiceWorker,'postMessage'>|null|undefined)=>void;
 onControllerChange:()=>void;
} {
 let expectingControllerChange=false;
 return {
  requestActivation(worker){
   if(!worker)return;
   expectingControllerChange=true;
   worker.postMessage({type:'SKIP_WAITING'});
  },
  onControllerChange(){
   if(!expectingControllerChange)return;
   expectingControllerChange=false;
   reload();
  },
 };
}

const serviceWorkerUpdateHandler=createServiceWorkerUpdateHandler(()=>globalThis.location.reload());

async function activateWaitingWorker(): Promise<void> {
 const registration=await navigator.serviceWorker?.getRegistration('/');
 serviceWorkerUpdateHandler.requestActivation(registration?.waiting);
}

async function registerServiceWorker(): Promise<void> {
 if(!('serviceWorker' in navigator))return;
 try{
  const registration=await navigator.serviceWorker.register('/sw.js',{scope:'/'});
  const announce=()=>snackbarQueue.push({key:'pwa-update',message:'A Somaris update is ready to install.',tone:'info',duration:8000,action:{label:'Install update',action:'pwa-update',icon:'refresh'}});
  if(registration.waiting&&navigator.serviceWorker.controller)announce();
  registration.addEventListener('updatefound',()=>{
   const worker=registration.installing;
   worker?.addEventListener('statechange',()=>{if(worker.state==='installed'&&navigator.serviceWorker.controller)announce();});
  });
  navigator.serviceWorker.addEventListener('controllerchange',serviceWorkerUpdateHandler.onControllerChange);
 }catch{/* Offline or unsupported browser: the app continues without installation support. */}
}

function patchCards(): void {
 if(!app)return;
 if(!['home','activity'].includes(page())){render();return;}
 const panelMode=document.documentElement.dataset.displayMode==='panel';
 const oldCards=[...app.querySelectorAll<HTMLElement>('[data-device-card]')];
 const expectedIds=state.managed.map(device=>device.enrollment.id);
 if(oldCards.map(card=>card.dataset.deviceCard).join('|')!==expectedIds.join('|')){render();return;}
 if(state.serviceError&&!state.managed.length&&!app.querySelector('.error-state')){render();return;}
 if(!state.serviceError&&app.querySelector('.error-state')){render();return;}
 for(const device of state.managed){
  const id=device.enrollment.id;
  const current=app.querySelector<HTMLElement>(`[data-device-card="${CSS.escape(id)}"]`);
  if(!current)continue;
  const nextTemplate=document.createElement('template');
  nextTemplate.innerHTML=renderManagedDeviceCard(device,state,panelMode);
  const next=nextTemplate.content.querySelector<HTMLElement>('[data-device-card]');
  if(!next)continue;
  if(current.dataset.room!==next.dataset.room){render();return;}
  for(const selector of ['.status-indicator','.blind-state']){
   const oldNode=current.querySelector(selector);
   const newNode=next.querySelector(selector);
   if(oldNode&&newNode)oldNode.replaceWith(newNode.cloneNode(true));
  }
  const oldGraphic=current.querySelector<HTMLElement>('[data-position-graphic]');
  const newGraphic=next.querySelector<HTMLElement>('[data-position-graphic]');
  if(oldGraphic&&newGraphic&&document.activeElement!==oldGraphic&&!positionInteraction){
   for(const name of ['aria-valuenow','aria-valuetext','data-movement']){
    const value=newGraphic.getAttribute(name);
    if(value===null)oldGraphic.removeAttribute(name);else oldGraphic.setAttribute(name,value);
   }
   const oldFill=oldGraphic.querySelector<HTMLElement>('.blind-art__fill');
   const newFill=newGraphic.querySelector<HTMLElement>('.blind-art__fill');
   if(oldFill&&newFill)oldFill.style.setProperty('--blind-fill',newFill.style.getPropertyValue('--blind-fill'));
  }
  const oldMessages=current.querySelector<HTMLElement>('[data-card-messages]');
  const newMessages=next.querySelector<HTMLElement>('[data-card-messages]');
  if(oldMessages&&newMessages)oldMessages.innerHTML=newMessages.innerHTML;
  const oldControls=[...current.querySelectorAll<HTMLButtonElement>('.controls button')];
  const newControls=[...next.querySelectorAll<HTMLButtonElement>('.controls button')];
  for(const oldControl of oldControls){
   const replacement=newControls.find(button=>button.dataset.action===oldControl.dataset.action);
   if(replacement)oldControl.disabled=replacement.disabled;
  }
  const oldRange=current.querySelector<HTMLInputElement>('[data-position-input]');
  const newRange=next.querySelector<HTMLInputElement>('[data-position-input]');
  if(oldRange&&newRange){
   oldRange.disabled=newRange.disabled;
   if(document.activeElement!==oldRange&&!positionInteraction)oldRange.value=newRange.value;
  }
  const oldOutput=current.querySelector<HTMLOutputElement>('[data-position-output]');
  const newOutput=next.querySelector<HTMLOutputElement>('[data-position-output]');
  if(oldOutput&&newOutput&&document.activeElement!==oldRange)oldOutput.value=newOutput.value;
 }
 updateTopbar();
}

function notifyCommandResult(id: string, command: CommandRecord): void {
 const [message]=commandMessage(command);
 const target=state.managed.find(device=>device.enrollment.id===id);
 const label=target?.enrollment.label;
 const tone=command.status==='write_acknowledged'?'success':['connection_failed','expired'].includes(command.status)?'error':'warning';
 const feedback=positionFeedback.get(id);
 if(command.action==='set_position'&&feedback){
  if(command.status==='write_acknowledged'){
   snackbarQueue.updateByKey(feedback.key,{message:'Command sent. Waiting for position feedback…',tone:'info',duration:0});
  }else{
   snackbarQueue.updateByKey(feedback.key,{message:label?`${label}: ${message}`:message,tone, duration:10000,action:['outcome_unknown','write_failed'].includes(command.status)?{label:'Refresh position',action:'refresh'}:null});
   positionFeedback.delete(id);
  }
  return;
 }
 const notification: Notification={key:`command:${command.id||id}:${command.status}`,message:label?`${label}: ${message}`:message,tone};
 if(['outcome_unknown','write_failed'].includes(command.status))notification.action={label:'Refresh position',action:'refresh'};
 snackbarQueue.push(notification);
}

function updatePositionFeedback():void {
 for(const [id,feedback] of positionFeedback){
  const device=state.managed.find(item=>item.enrollment.id===id);
  const observedAt=Date.parse(device?.position_observed_at||'');
  if(!device||device.position_freshness!=='fresh'||!Number.isFinite(observedAt)||observedAt<feedback.acceptedAt)continue;
  if(device.position?.current_closed_percent!==feedback.targetClosedPercent)continue;
  snackbarQueue.updateByKey(feedback.key,{message:`${feedback.label} reached ${feedback.openPercent}% open`,tone:'success',duration:10000});
  if(positionUndo?.id===id)positionUndo.expires=Date.now()+10000;
  positionFeedback.delete(id);
 }
}
export function buildCommandRequestBody(
 device: ManagedDevice,
 action: import('./types.mjs').BlindAction,
 positionClosedPercent?: number,
 motorSpeedPercent?: number,
): Record<string, unknown> {
 const expectedLabel=device.enrollment.label;
 if(typeof expectedLabel!=='string'||!expectedLabel.trim()||expectedLabel.length>64)throw new Error('Refresh the device list before sending a command.');
 const body:Record<string,unknown>={action,expected_label:expectedLabel};
 if(action==='set_position')body.position_closed_percent=positionClosedPercent;
 if(action==='set_motor_speed')body.motor_speed_percent=motorSpeedPercent;
 return body;
}
async function sendCommand(id: string, action: import('./types.mjs').BlindAction, positionClosedPercent?: number, motorSpeedPercent?: number): Promise<void> {
 if((state.busy.has(id)||state.reconciliationRequired.has(id)||blindBatchIsLocked(state.batch)) && action!=='stop')return;
 const target=state.managed.find(device=>device.enrollment.id===id);
 if(!target?.enrollment.label){snackbarQueue.push({key:`command-target:${id}`,message:'Refresh the blind list before sending a command.',tone:'warning'});render();return;}
 const previousTarget=target.position?.target_closed_percent??target.position?.current_closed_percent;
 const requestStartedAt=Date.now();
 invalidateUndoFor(id);
 const token=Symbol();
 state.busy.add(id);
 state.tokens[id]=token;
 state.commands[id]={action,status:'queued'};
 render();
 try{
  const key=globalThis.crypto?.randomUUID?.()||`${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const body=buildCommandRequestBody(target,action,positionClosedPercent,motorSpeedPercent);
  const command=await api<CommandRecord>(`/api/control/v1/devices/${encodeURIComponent(id)}/commands`,{method:'POST',headers:{'Content-Type':'application/json','Idempotency-Key':key},body:JSON.stringify(body)});
  if(!command.id)throw new Error('The service returned a command without an ID.');
  if(state.tokens[id]!==token)return;
  state.commands[id]=command;
  if(action==='set_position'&&typeof positionClosedPercent==='number'){
   const openPercent=openPercentFromClosedPercent(positionClosedPercent);
   const key=`position-feedback:${id}`;
   const acceptedAt=Date.parse(command.accepted_at||'')||requestStartedAt;
   positionFeedback.set(id,{key,targetClosedPercent:positionClosedPercent,openPercent,label:target.enrollment.label,acceptedAt});
   const canUndo=typeof previousTarget==='number'&&Number.isInteger(previousTarget)&&previousTarget>=0&&previousTarget<=100;
   const expires=Date.now()+10000;
   if(canUndo)positionUndo={id,previous:previousTarget,commandToken:token,expires};
   snackbarQueue.push({key,message:positionCommandMessage(openPercent),tone:'info',duration:0,action:canUndo?{label:`Undo position for ${target.enrollment.label}`,action:`undo-position:${encodeURIComponent(id)}:${previousTarget}`,icon:'undo'}:null});
   updatePositionFeedback();
  }
  render();
  pollCommand(id,command.id,token);
 }catch(error){
  if(state.tokens[id]!==token)return;
  state.commands[id]={status:'outcome_unknown'};
  positionUndo=null;
  snackbarQueue.push({key:`command-request:${id}`,message:`Command request failed: ${errorMessage(error)}. Its outcome may be unknown; check telemetry before trying again.`,tone:'warning',action:{label:'Refresh position',action:'refresh',icon:'refresh'}});
  if(!state.reconciliationRequired.has(id))state.busy.delete(id);
  render();
 }
}
async function pollCommand(id: string, commandId: string, token: symbol): Promise<void> {
 try{
  const command=await api<CommandRecord>(`/api/control/v1/commands/${encodeURIComponent(commandId)}`);
  if(state.tokens[id]!==token)return;
  state.commands[id]=command;
  render();
  if(!terminal.has(command.status)){setTimeout(()=>pollCommand(id,commandId,token),1500);return;}
  if(['outcome_unknown','write_failed'].includes(command.status))state.reconciliationRequired.add(id);
  if(!state.reconciliationRequired.has(id))state.busy.delete(id);
  notifyCommandResult(id,command);
  await refresh(true);
 }catch(error){await handleCommandPollFailure(state,id,token,error,()=>refreshCommandTelemetry(state,api,id),render);}
}
function pollBlindBatch(batch: BatchState): void {
 setTimeout(async()=>{
  if(state.batch!==batch || !batch.id || !blindBatchIsLocked(batch))return;
  const updated=await checkBlindBatch(state,api,render);
  if(!updated || state.batch!==batch)return;
  if(!blindBatchIsLocked(batch)){completeBatchUndo(batch);await refresh();return;}
  pollBlindBatch(batch);
 },1500);
}
async function startBlindBatch(action: BatchAction): Promise<void> {
 const previous=new Map<string,number>();
 for(const device of state.managed){
  const position=device.position?.target_closed_percent??device.position?.current_closed_percent;
  if(device.availability==='ready'&&device.calibration_mode_active!==true&&typeof position==='number'&&Number.isInteger(position)&&position>=0&&position<=100)previous.set(device.enrollment.id,position);
 }
 batchUndo={id:'pending',previous,expires:Date.now()+120000};
 const accepted=await submitBlindBatch(state,api,action,render);
 if(!accepted){if(state.batch?.status==='rejected')await refresh();return;}
 const batch=state.batch;
 if(batch?.id && blindBatchIsLocked(batch))pollBlindBatch(batch);
 else {if(batch)completeBatchUndo(batch);await refresh();}
}
async function retryBlindBatchCheck(): Promise<void> {
 const batch=state.batch;
 if(!batch)return;
 const updated=await checkBlindBatch(state,api,render);
 if(!updated || state.batch!==batch)return;
 if(blindBatchIsLocked(batch) && batch.id)pollBlindBatch(batch);
 else if(!blindBatchIsLocked(batch)){completeBatchUndo(batch);await refresh();}
}

function completeBatchUndo(batch: BatchState): void {
 if(!['completed','partial'].includes(batch.status)||!batchUndo?.previous.size)return;
 const result=batch.data||batch;
 const acknowledged=new Set((result.targets||[]).filter(target=>target.status==='write_acknowledged').map(target=>target.managed_device_id).filter((id):id is string=>typeof id==='string'));
 const previous=new Map([...batchUndo.previous].filter(([id])=>acknowledged.has(id)));
 if(!previous.size)return;
 const expires=Date.now()+10000;
 batchUndo={id:batch.id||'batch',previous,expires};
 state.notify({key:`batch-undo:${batch.id||'last'}`,message:`${batch.action==='open'?'Open':'Close'} all finished. Restore the previous targets?`,tone:'success',duration:10000,action:{label:'Undo all-blinds command',action:'undo-batch',icon:'undo'}});
}
async function simulatorReset(id: string): Promise<void> {try{await api(`/api/dev/simulators/${encodeURIComponent(id)}/reset`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({position_closed_percent:100})});await refresh();snackbarQueue.push({message:'Simulator reset.',tone:'success'});}catch(error){snackbarQueue.push({message:`Simulator reset failed: ${errorMessage(error)}`,tone:'error'});render();}}
async function confirmEnrollmentRemoval(id: string): Promise<void> {
 if(state.removingEnrollmentId===id)return;
 const removal=removeEnrolledBlind(state,api,()=>refreshService(state,api),id);
 render();
 if(await removal){ location.hash='home'; return; }
 render();
}

function closestHtmlElement(target: EventTarget | null, selector: string): HTMLElement | null {
 return target instanceof Element ? target.closest<HTMLElement>(selector) : null;
}

function containsRelatedTarget(element: HTMLElement, target: EventTarget | null): boolean {
 return target instanceof Node && element.contains(target);
}

export function parseActionValue(value: string | undefined): [string, string, string] {
 const [action='', argument='', id=''] = value?.split(':') ?? [];
 return [action, argument, id];
}

function isBlindAction(action: string | undefined): action is import('./types.mjs').BlindAction {
 return action !== undefined && [
  'open','close','stop','toggle','set_position','set_motor_speed',
  'calibration_start','calibration_nudge_open','calibration_set_open_limit',
  'calibration_nudge_closed','calibration_set_closed_limit','calibration_stop',
 ].includes(action);
}

export function dispatchBlindCommand(
 action: string,
 encodedId: string,
 send: (id: string, action: import('./types.mjs').BlindAction) => unknown = sendCommand,
): boolean {
 if(!encodedId || !isBlindAction(action))return false;
 void send(decodeURIComponent(encodedId),action);
 return true;
}

function changeDisplayMode(preference: string, applyDisplayMode: () => boolean): void {
 if(!displayModes.has(preference as DisplayPreference))return;
 displayPreference=preference as DisplayPreference;
 try {
  globalThis.localStorage?.setItem('somaris-display-mode',displayPreference);
 } catch { /* The current selection still applies until the page reloads. */ }
 applyDisplayMode();
 render();
 document.getElementById('layout-mode-toggle')?.focus({preventScroll:true});
}

function changeTheme(preference: string, focusTargetId='theme-toggle'): void {
 if(!isThemePreference(preference))return;
 themePreference=preference;
 try { globalThis.localStorage?.setItem('somaris-theme',themePreference); } catch { /* Current selection still applies for this page. */ }
 render();
 document.getElementById(focusTargetId)?.focus({preventScroll:true});
}

function toggleTheme():void {
 changeTheme(themePreference==='dark'?'light':'dark');
}

function handleBatchClick(argument: string): boolean {
 if(argument==='check')void retryBlindBatchCheck();
 else if(argument==='open'||argument==='close')void startBlindBatch(argument);
 return true;
}

function handlePositionClick(argument: string, id: string): boolean {
 if(document.documentElement.dataset.displayMode==='panel')return false;
 const position=Number(id);
 if(Number.isInteger(position)&&position>=0&&position<=100){dismissPositionCue();void sendCommand(argument,'set_position',closedPercentFromOpenPercent(position));}
 return true;
}

function handleRemovalClick(action: string, deviceId: string): boolean {
 if(action==='remove'){
  state.confirmingRemovalId=deviceId;
  state.pendingRemovalFocusId=deviceId;
  render();
  return true;
 }
 if(action==='remove-confirm'){
  void confirmEnrollmentRemoval(deviceId);
  return true;
 }
 if(action==='remove-cancel' && state.removingEnrollmentId!==deviceId){
  state.confirmingRemovalId=null;
  state.pendingRemovalFocusId=null;
  render();
 }
 return true;
}

function handleRoomRemovalClick(room:string,confirm:boolean):void{
 if(!confirm){
  confirmingRoomRemoval=room;
  editingRoomName=null;
  render();
  return;
 }
 void removeRoom(state,api,()=>refreshService(state,api),room).then(removed=>{
  if(removed)confirmingRoomRemoval=null;
  render();
 });
}

type ClickActionHandler = (
 argument: string,
 id: string,
 deviceId: string,
 applyDisplayMode: () => boolean,
) => boolean;

const clickActionHandlers: Record<string, ClickActionHandler> = {
 'snackbar-dismiss':(_argument,_id,deviceId)=>{snackbarQueue.dismiss(deviceId);return true;},
 'layout-cycle':(_argument,_id,_deviceId,applyDisplayMode)=>{changeDisplayMode(nextDisplayPreference(displayPreference),applyDisplayMode);return true;},
 'theme-toggle':()=>{toggleTheme();return true;},
 command:(argument,id)=>dispatchBlindCommand(argument,id),
 batch:argument=>handleBatchClick(argument),
 position:(_argument,id,deviceId)=>handlePositionClick(deviceId,id),
 'activity-toggle':()=>{const expanded=activityCollapsedByUser??document.documentElement.dataset.displayMode!=='mobile';activityCollapsedByUser=!expanded;render();return true;},
 'demo-hint-dismiss':()=>{hintDismissed=true;try{globalThis.localStorage?.setItem('somaris-demo-hint-dismissed','true');}catch{}render();return true;},
 'undo-position':(_argument,id,deviceId)=>{const position=Number(id);if(positionUndo&&positionUndo.id===deviceId&&positionUndo.expires>Date.now()&&state.tokens[deviceId]===positionUndo.commandToken)void sendCommand(deviceId,'set_position',position);return true;},
 'undo-batch':()=>{void undoBatchTargets();return true;},
 'pwa-update':()=>{void activateWaitingWorker();return true;},
 details:argument=>{
  if(!argument)return false;
  location.hash=`details/${encodeURIComponent(argument)}`;
  return true;
 },
 refresh:()=>{void refresh();return true;},
 'sim-reset':argument=>{
  if(!argument)return false;
  void simulatorReset(argument);
  return true;
 },
 'demo-reset':()=>{
  void resetDemo(
   state,
   api,
   ()=>refreshService(state,api),
   ()=>globalThis.confirm('Reset your demo? This restores all four blinds and clears this session’s activity.'),
  ).then(()=>render());
  return true;
 },
 remove:(_argument,_id,deviceId)=>handleRemovalClick('remove',deviceId),
 'remove-confirm':(_argument,_id,deviceId)=>handleRemovalClick('remove-confirm',deviceId),
 'remove-cancel':(_argument,_id,deviceId)=>handleRemovalClick('remove-cancel',deviceId),
 'room-rename':(_argument,_id,room)=>{
  editingRoomName=room;
  confirmingRoomRemoval=null;
  render();
  app?.querySelector<HTMLInputElement>('[data-room-name-input]')?.focus();
  return true;
 },
 'room-edit-cancel':()=>{editingRoomName=null;render();return true;},
 'room-remove':(_argument,_id,room)=>{handleRoomRemovalClick(room,false);return true;},
 'room-remove-confirm':(_argument,_id,room)=>{handleRoomRemovalClick(room,true);return true;},
 'room-remove-cancel':()=>{confirmingRoomRemoval=null;render();return true;},
};

function dispatchClickAction(action: string, argument: string, id: string, applyDisplayMode: () => boolean): void {
 const deviceId=argument ? decodeURIComponent(argument) : '';
 const handler=clickActionHandlers[action];
 if(!handler || !handler(argument,id,deviceId,applyDisplayMode))location.hash=action;
}

async function saveSimulatorSettings(form: HTMLFormElement): Promise<void> {
 const id=form.dataset.simulator;
 if(!id)return;
 const values=Object.fromEntries(new FormData(form).entries());
 try{
  await api(`/api/dev/simulators/${encodeURIComponent(id)}/scenario`,{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify(values)});
  await refresh();
  snackbarQueue.push({message:'Simulator settings saved.',tone:'success'});
 }catch(error){
  snackbarQueue.push({message:`Simulator settings could not be saved: ${errorMessage(error)}`,tone:'error'});
  render();
 }
}

function submitMotorSpeed(form: HTMLFormElement): void {
 const id=form.dataset.motorSpeed;
 if(!id)return;
 const speed=Number(new FormData(form).get('motor_speed_percent'));
 if(Number.isInteger(speed)&&speed>=0&&speed<=100)void sendCommand(id,'set_motor_speed',undefined,speed);
}

async function submitEnrollmentForm(form: HTMLFormElement): Promise<void> {
 const button=form.querySelector<HTMLButtonElement>('button');
 const id=form.dataset.enroll;
 const input=form.querySelector<HTMLInputElement>('[data-display-name]');
 const labelValue=new FormData(form).get('label');
 const label=typeof labelValue==='string' ? labelValue.trim() : '';
 const validation=validateDisplayName(label);
 const error=input?.parentElement?.querySelector<HTMLElement>('.field-error');
 if(validation){
  input?.setAttribute('aria-invalid','true');
  if(error){error.textContent=validation;error.hidden=false;}
  input?.focus();
  return;
 }
 const roomValue=roomValueFromForm(form);
 showRoomValueError(form,roomValue.error);
 if(roomValue.error){
  const input=form.querySelector<HTMLInputElement>('[data-room-custom-input]');
  const select=form.querySelector<HTMLSelectElement>('[data-room-choice]');
  (select?.value==='custom'?input:select)?.focus();
  return;
 }
 input?.setAttribute('aria-invalid','false');
 if(error){error.textContent='';error.hidden=true;}
 if(!id || !label)return;
 if(button){button.disabled=true;button.innerHTML=renderIcon('spinner');button.setAttribute('aria-label',`Adding ${label}`);button.title=`Adding ${label}`;}
 const added=await enrollBlind(state,api,()=>refreshService(state,api),id,label,roomValue.room);
 if(added)location.hash='home';
 render();
}

function roomValueFromForm(form:HTMLFormElement):{room:string|null;error:string}{
 const data=new FormData(form);
 const choice=String(data.get('room_choice')||'unassigned');
 const custom=String(data.get('room_custom')||'');
 if(choice==='unassigned')return {room:null,error:''};
 const room=roomFromChoice(choice,custom);
 if(room===null)return {room:null,error:'Choose a room or Unassigned.'};
 return {room:room.trim(),error:validateRoomName(room)};
}

function showRoomValueError(form:HTMLFormElement,message:string):void{
 const input=form.querySelector<HTMLInputElement>('[data-room-custom-input]');
 const error=form.querySelector<HTMLElement>('[data-room-error]');
 const customSelected=form.querySelector<HTMLSelectElement>('[data-room-choice]')?.value==='custom';
 input?.setAttribute('aria-invalid',String(customSelected&&!!message));
 if(error){error.textContent=message;error.hidden=!message;}
}

async function submitRoomAssignmentForm(form:HTMLFormElement):Promise<void>{
 const encodedId=form.dataset.roomAssignment;
 if(!encodedId)return;
 const selected=roomValueFromForm(form);
 showRoomValueError(form,selected.error);
 if(selected.error){
  const input=form.querySelector<HTMLInputElement>('[data-room-custom-input]');
  const select=form.querySelector<HTMLSelectElement>('[data-room-choice]');
  (select?.value==='custom'?input:select)?.focus();
  return;
 }
 const button=form.querySelector<HTMLButtonElement>('button[type="submit"]');
 if(button)button.disabled=true;
 await updateBlindRoom(state,api,()=>refreshService(state,api),decodeURIComponent(encodedId),selected.room);
 render();
}

async function submitRoomRenameForm(form:HTMLFormElement):Promise<void>{
 const encodedRoom=form.dataset.roomRename;
 const input=form.querySelector<HTMLInputElement>('[data-room-name-input]');
 const name=input?.value.trim()||'';
 const validation=validateRoomName(name);
 const error=form.querySelector<HTMLElement>('[data-room-name-error]');
 input?.setAttribute('aria-invalid',String(!!validation));
 if(error){error.textContent=validation;error.hidden=!validation;}
 if(validation){input?.focus();return;}
 const button=form.querySelector<HTMLButtonElement>('button[type="submit"]');
 if(button)button.disabled=true;
 const renamed=await renameRoom(state,api,()=>refreshService(state,api),decodeURIComponent(encodedRoom||''),name);
 if(renamed)editingRoomName=null;
 render();
}

async function handleFormSubmit(event: SubmitEvent): Promise<void> {
 const form=event.target instanceof HTMLFormElement ? event.target : null;
 if(!form)return;
 if(form.matches('[data-simulator]')){
  event.preventDefault();
  await saveSimulatorSettings(form);
  return;
 }
 if(form.matches('[data-motor-speed]')){
  event.preventDefault();
  submitMotorSpeed(form);
  return;
 }
 if(form.matches('[data-room-assignment]')){
  event.preventDefault();
  await submitRoomAssignmentForm(form);
  return;
 }
 if(form.matches('[data-room-rename]')){
  event.preventDefault();
  await submitRoomRenameForm(form);
  return;
 }
 if(!form.matches('[data-enroll]'))return;
 event.preventDefault();
 await submitEnrollmentForm(form);
}

if(app){
 const actionRoot=document.querySelector<HTMLElement>('.app-shell');
 if(actionRoot){
 const clockSlot=document.querySelector<HTMLElement>('#header-clock-slot');
 if(clockSlot)mountClock(clockSlot);
 const applyDisplayMode = () => {
  const previous = document.documentElement.dataset.displayMode;
  document.documentElement.dataset.displayMode = resolveDisplayMode(displayPreference, window.innerWidth, window.innerHeight);
  return previous !== document.documentElement.dataset.displayMode;
 };
 applyDisplayMode();
 void registerServiceWorker();
 let initialized=false;
 window.addEventListener('resize', () => { if (applyDisplayMode()&&initialized) render(); });
 window.addEventListener('hashchange',()=>{
  routeScrollPositions.set(lastRenderedRoute,window.scrollY);
  const next=page();
  if(initialized){render();window.scrollTo({top:routeScrollPositions.get(next)??(next==='activity'?document.getElementById('activity')?.offsetTop??0:0),behavior:'instant'});}
  lastRenderedRoute=next;
 });
 window.addEventListener('offline',()=>{serviceFailure(state,new Error('Network unavailable'));render();});
 window.addEventListener('online',()=>{void refresh();});
 document.addEventListener('keydown',(event:KeyboardEvent)=>{
  const graphic=closestHtmlElement(event.target,'[data-position-graphic]');
  const range=closestHtmlElement(event.target,'[data-position-input]');
  if(!graphic&&!(range instanceof HTMLInputElement))return;
  const id=graphic?.dataset.positionGraphic||((range as HTMLInputElement).dataset.positionInput)||'';
  const current=graphic?Number(graphic.getAttribute('aria-valuenow')||0):Number((range as HTMLInputElement).value);
  let next=current;
  const step=event.shiftKey?10:1;
  if(event.key==='ArrowUp'||event.key==='ArrowRight')next+=step;
  else if(event.key==='ArrowDown'||event.key==='ArrowLeft')next-=step;
  else if(event.key==='Home')next=0;
  else if(event.key==='End')next=100;
  else if(event.key==='Enter'){event.preventDefault();commitPosition(id,current);return;}
  else return;
  event.preventDefault();
  if(graphic)dismissPositionCue();
  if(!positionInteraction)beginPositionInteraction(id,current);
  if(graphic)previewPosition(id,next);
 });
 document.addEventListener('keyup',(event:KeyboardEvent)=>{
  if(!['ArrowUp','ArrowRight','ArrowDown','ArrowLeft','Home','End'].includes(event.key))return;
  const graphic=closestHtmlElement(event.target,'[data-position-graphic]');
  const range=closestHtmlElement(event.target,'[data-position-input]');
  if(graphic){const id=graphic.dataset.positionGraphic||'';commitPosition(id,Number(graphic.getAttribute('aria-valuenow')||0));}
  else if(range instanceof HTMLInputElement){const id=range.dataset.positionInput||'';commitPosition(id,Number(range.value));}
 });
 actionRoot.addEventListener('click', (event: MouseEvent)=>{
  const target=closestHtmlElement(event.target,'[data-action]');
  if(!target)return;
  const actionValue=target.dataset.action;
  const [action,argument,id]=parseActionValue(actionValue);
  dispatchClickAction(action,argument,id,applyDisplayMode);
 });
 actionRoot.addEventListener('pointerdown',(event:PointerEvent)=>{
  const graphic=closestHtmlElement(event.target,'[data-position-graphic]');
  const range=closestHtmlElement(event.target,'[data-position-input]');
  if(graphic){const id=graphic.dataset.positionGraphic||'';const value=Number(graphic.getAttribute('aria-valuenow')||0);dismissPositionCue();beginPositionInteraction(id,value);graphic.setPointerCapture?.(event.pointerId);previewPosition(id,graphicValueFromPointer(graphic,event.clientY));}
  else if(range instanceof HTMLInputElement){const id=range.dataset.positionInput||'';const device=state.managed.find(item=>item.enrollment.id===id);const closed=device?.position?.target_closed_percent??device?.position?.current_closed_percent;range.setPointerCapture?.(event.pointerId);beginPositionInteraction(id,typeof closed==='number'?openPercentFromClosedPercent(closed):Number(range.value));}
 });
 actionRoot.addEventListener('pointermove',(event:PointerEvent)=>{
  if(!positionInteraction||!positionInteractionId)return;
  const id=positionInteractionId;
  const graphic=app?.querySelector<HTMLElement>(`[data-position-graphic="${CSS.escape(id)}"]`);
  const range=app?.querySelector<HTMLInputElement>(`[data-position-input="${CSS.escape(id)}"]`);
  if(graphic)previewPosition(id,graphicValueFromPointer(graphic,event.clientY));
  else if(range)previewPosition(id,Number(range.value));
 });
 actionRoot.addEventListener('pointerup',(event:PointerEvent)=>{
  if(!positionInteraction||!positionInteractionId)return;
  const id=positionInteractionId;
  const graphic=app?.querySelector<HTMLElement>(`[data-position-graphic="${CSS.escape(id)}"]`);
  const range=app?.querySelector<HTMLInputElement>(`[data-position-input="${CSS.escape(id)}"]`);
  const value=graphic?graphicValueFromPointer(graphic,event.clientY):Number(range?.value||0);
  previewPosition(id,value);
  commitPosition(id,value);
 });
 actionRoot.addEventListener('pointercancel',()=>{if(positionInteractionId)cancelPositionInteraction(positionInteractionId);});
 snackbarRoot?.addEventListener('pointerover',(event: PointerEvent)=>{
  const item=closestHtmlElement(event.target,'[data-snackbar-id]');
  if(item&&!containsRelatedTarget(item,event.relatedTarget))snackbarQueue.pause(item.dataset.snackbarId ?? '','pointer');
 });
 snackbarRoot?.addEventListener('pointerout',(event: PointerEvent)=>{
  const item=closestHtmlElement(event.target,'[data-snackbar-id]');
  if(item&&!containsRelatedTarget(item,event.relatedTarget))snackbarQueue.resume(item.dataset.snackbarId ?? '','pointer');
 });
 snackbarRoot?.addEventListener('focusin',(event: FocusEvent)=>{
  const item=closestHtmlElement(event.target,'[data-snackbar-id]');
  if(item)snackbarQueue.pause(item.dataset.snackbarId ?? '','focus');
 });
 snackbarRoot?.addEventListener('focusout',(event: FocusEvent)=>{
  const item=closestHtmlElement(event.target,'[data-snackbar-id]');
  if(item&&!containsRelatedTarget(item,event.relatedTarget))snackbarQueue.resume(item.dataset.snackbarId ?? '','focus');
 });
 actionRoot.addEventListener('input',(event: Event)=>{
  const nameInput=closestHtmlElement(event.target,'[data-display-name]');
  if(nameInput instanceof HTMLInputElement){
   if(nameInput.getAttribute('aria-invalid')==='true'){
    const validation=validateDisplayName(nameInput.value);
    const error=nameInput.parentElement?.querySelector<HTMLElement>('.field-error');
    if(error){error.textContent=validation;error.hidden=!validation;}
    nameInput.setAttribute('aria-invalid',String(!!validation));
   }
   return;
  }
  const customRoomInput=closestHtmlElement(event.target,'[data-room-custom-input]');
  if(customRoomInput instanceof HTMLInputElement){
   if(customRoomInput.getAttribute('aria-invalid')==='true'){
    const form=customRoomInput.closest('form');
    if(form)showRoomValueError(form,validateRoomName(customRoomInput.value));
   }
   return;
  }
  const renamedRoomInput=closestHtmlElement(event.target,'[data-room-name-input]');
  if(renamedRoomInput instanceof HTMLInputElement&&renamedRoomInput.getAttribute('aria-invalid')==='true'){
   const validation=validateRoomName(renamedRoomInput.value);
   const error=renamedRoomInput.parentElement?.querySelector<HTMLElement>('[data-room-name-error]');
   if(error){error.textContent=validation;error.hidden=!validation;}
   renamedRoomInput.setAttribute('aria-invalid',String(!!validation));
   return;
  }
  const input=closestHtmlElement(event.target,'[data-position-input]');
  if(!input)return;
  const range=input instanceof HTMLInputElement ? input : null;
  if(!range)return;
  const id=range.dataset.positionInput||'';
  if(!positionInteraction){const device=state.managed.find(item=>item.enrollment.id===id);const closed=device?.position?.target_closed_percent??device?.position?.current_closed_percent;beginPositionInteraction(id,typeof closed==='number'?openPercentFromClosedPercent(closed):Number(range.value));}
  previewPosition(id,Number(range.value));
 });
 app.addEventListener('change',(event:Event)=>{
  const themePreferenceControl=closestHtmlElement(event.target,'[data-theme-preference]');
  if(themePreferenceControl instanceof HTMLSelectElement){
   changeTheme(themePreferenceControl.value,themePreferenceControl.id);
   return;
  }
  const roomChoice=closestHtmlElement(event.target,'[data-room-choice]');
  if(roomChoice instanceof HTMLSelectElement){
   const picker=roomChoice.closest('.room-picker');
   const customField=picker?.querySelector<HTMLElement>('[data-room-custom-field]');
   if(customField)customField.hidden=roomChoice.value!=='custom';
   if(roomChoice.value!=='custom'){
    const input=picker?.querySelector<HTMLInputElement>('[data-room-custom-input]');
    input?.setAttribute('aria-invalid','false');
    const error=picker?.querySelector<HTMLElement>('[data-room-error]');
    if(error){error.textContent='';error.hidden=true;}
   }
   return;
  }
  const range=closestHtmlElement(event.target,'[data-position-input]');
  if(range instanceof HTMLInputElement){const id=range.dataset.positionInput||'';if(!positionInteraction)beginPositionInteraction(id,Number(range.value));commitPosition(id,Number(range.value));}
 });
 app.addEventListener('submit',(event: SubmitEvent)=>void handleFormSubmit(event));
 const initializeDashboard=async()=>{
  try{
   const mode=await api<{demo_mode:boolean}>('/api/v1/demo');
   state.demoMode=mode.demo_mode===true;
   if(state.demoMode)await api('/api/demo/session',{method:'POST'});
   await refresh();
  }catch(error){serviceFailure(state,error);render();}
  finally{initialized=true;}
 };
 void initializeDashboard();
 setInterval(()=>{if(initialized)void refresh(true);},5000);
 }
}
