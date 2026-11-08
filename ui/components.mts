import type {
  Availability,
  BatchAction,
  BatchResponse,
  BatchState,
  CommandStatus,
  DisplayPreference,
  SnackbarAction,
  SnackbarItem,
  Tone,
  VisualTone,
} from './types.mjs';

export const escapeHtml = (value: unknown): string => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;',
}[character] ?? character));

const icons: Record<string, string> = {
  add:'<path d="M12 5v14M5 12h14"/>',
  display:'<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/>',
  observed:'<circle cx="12" cy="12" r="8.5"/><path d="m8.5 12 2.3 2.3 4.8-5"/>',
  stale:'<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3 2"/>',
  not_seen:'<circle cx="12" cy="12" r="8.5"/><path d="m9 9 6 6m0-6-6 6"/>',
  ambiguous:'<path d="M12 3.5 22 20H2L12 3.5Z"/><path d="M12 9v5m0 3h.01"/>',
  unavailable:'<circle cx="12" cy="12" r="8.5"/><path d="M9.8 9a2.3 2.3 0 1 1 3.9 1.7c-1 .8-1.7 1.2-1.7 2.8m0 3h.01"/>',
  refresh:'<path d="M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4"/>',
  open:'<path d="M12 19V5M5 12l7-7 7 7"/>',
  stop:'<rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor" stroke="none"/>',
  close:'<path d="M12 5v14m7-7-7 7-7-7"/>',
  trash:'<path d="M4 7h16M10 11v6m4-6v6M5 7l1 14h12l1-14M9 7V4h6v3"/>',
};
type IconName = keyof typeof icons;
const availabilityIcons: Partial<Record<Availability, IconName>> = {ready:'observed', stale:'stale', not_seen:'not_seen', ambiguous:'ambiguous'};

export const DISPLAY_MODE_OPTIONS = [
  ['auto', 'Auto'],
  ['mobile', 'Mobile'],
  ['panel', 'Panel'],
  ['desktop', 'Desktop'],
] as const satisfies readonly (readonly [DisplayPreference, string])[];

interface BlindStatusOptions {
  positionPercent: number | null | undefined;
  movementStatus: MovementStatus;
  compact?: boolean;
  panelMode?: boolean;
}

type MovementStatus = 'opening' | 'closing' | 'at_target' | 'unavailable';

interface BadgeOptions {
  label: string;
  tone?: VisualTone;
}

interface StatusIndicatorOptions {
  label?: string;
  tone?: VisualTone;
  status?: Availability;
}

interface DeviceCardHeaderOptions {
  id: string;
  label: string;
  availabilityLabel: string;
  availabilityTone?: VisualTone;
  availabilityStatus?: Availability;
  isSimulated?: boolean;
  lastSeen?: string;
}

interface ButtonOptions {
  label: string;
  action: string;
  variant?: string;
  disabled?: boolean;
  icon?: string | null;
  iconOnly?: boolean;
  accessibleLabel?: string;
  title?: string;
}

interface TopbarActionsOptions {
  blindCount?: number;
  panelMode?: boolean;
  displayPreference?: string;
  serviceLabel?: string;
  serviceTone?: VisualTone;
}

interface BlindActionsOptions {
  deviceId: string;
  blindLabel: string;
  panelMode?: boolean;
  disabled?: boolean;
  busy?: boolean;
  movementDisabled?: boolean;
  supportsPosition?: boolean;
  positionClosedPercent?: number;
}

interface BatchFeedbackView {
  id?: string | null;
  action?: BatchAction;
  status?: string;
  data?: Partial<BatchResponse> | null;
  error?: string | null;
  target_count?: number;
  acknowledged_count?: number;
  failed_count?: number;
  pending_count?: number;
  skipped_count?: number;
  skipped?: BatchResponse['skipped'];
  targets?: BatchResponse['targets'];
}

interface IconButtonOptions {
  label: string;
  action: string;
  icon: string;
  variant?: string;
  disabled?: boolean;
}

interface BlindCardOptions extends DeviceCardHeaderOptions, BlindStatusOptions {
  blindLines?: number;
  controlsMarkup: string;
  messageMarkup?: string;
  footerMarkup?: string;
}

interface SnackbarRenderOptions {
  id?: string;
  message?: string;
  tone?: Tone;
  action?: SnackbarAction | null;
  details?: string[];
}

interface BatchControlsOptions {
  blindCount?: number;
  eligibleCount?: number;
  disabled?: boolean;
  busy?: boolean;
  panelMode?: boolean;
  batch?: BatchState | BatchResponse | null;
}

interface DeviceListOptions {
  count: number;
  panelMode?: boolean;
  cardsMarkup?: string;
  emptyMarkup?: string;
  toolbarMarkup?: string;
  title?: string;
}

interface InfoItemOptions {
  label: string;
  value?: string | number | null;
  contentMarkup?: string;
}

export function iconMarkup(name: string | null | undefined): string {
  if (!name) return '';
  const paths = icons[name];
  return paths
    ? `<svg class="ui-icon" data-icon="${escapeHtml(name)}" viewBox="0 0 24 24" aria-hidden="true" focusable="false">${paths}</svg>`
    : '';
}

export function blindState(positionPercent: number | null | undefined, movementStatus: MovementStatus): {label: string; percentClosed: number | null} {
  const percentClosed = isPercent(positionPercent)
    ? positionPercent
    : null;
  if (movementStatus === 'opening') return {label:'Opening', percentClosed};
  if (movementStatus === 'closing') return {label:'Closing', percentClosed};
  if (percentClosed === null) return {label:'Position unavailable', percentClosed:null};
  if (percentClosed === 0) return {label:'Open', percentClosed};
  if (percentClosed === 100) return {label:'Closed', percentClosed};
  return {label:'Partially open', percentClosed};
}

export function movementStatusFromPositions(currentPercent: number | null | undefined, targetPercent: number | null | undefined): MovementStatus {
  const validCurrent = isPercent(currentPercent);
  const validTarget = isPercent(targetPercent);
  if (!validCurrent || !validTarget) return 'unavailable';
  if (currentPercent === targetPercent) return 'at_target';
  return targetPercent > currentPercent ? 'closing' : 'opening';
}

function isPercent(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 100;
}

export function renderBlindStatus({positionPercent, movementStatus, compact = false, panelMode = false}: BlindStatusOptions): string {
  const {label, percentClosed} = blindState(positionPercent, movementStatus);
  const variants = [compact && 'blind-state--compact', panelMode && 'blind-state--panel'].filter(Boolean).join(' ');
  return `<div class="blind-state${variants ? ` ${variants}` : ''}"><strong class="blind-state__label">${escapeHtml(label)}</strong>${percentClosed === null ? '' : `<span class="blind-state__percent">${percentClosed}% closed</span>`}</div>`;
}

export function renderBadge({label, tone = ''}: BadgeOptions): string {
  const variant = tone ? ` ${escapeHtml(tone)}` : '';
  return `<span class="badge${variant}">${escapeHtml(label)}</span>`;
}

export function renderStatusIndicator({label, tone = '', status = 'unavailable'}: StatusIndicatorOptions = {}): string {
  const icon = availabilityIcons[status] || 'unavailable';
  const toneClass = tone ? ` ${escapeHtml(tone)}` : '';
  const name = escapeHtml(label || 'Availability unavailable');
  return `<span class="status-indicator${toneClass}" role="img" aria-label="${name}" title="${name}">${iconMarkup(icon)}</span>`;
}

export function renderDisplayModeToggle({displayPreference = 'auto'}: {displayPreference?: string} = {}): string {
  const selectedIndex = DISPLAY_MODE_OPTIONS.findIndex(([value]) => value === displayPreference);
  const safeIndex = selectedIndex < 0 ? 0 : selectedIndex;
  const [, currentLabel] = DISPLAY_MODE_OPTIONS[safeIndex] ?? DISPLAY_MODE_OPTIONS[0];
  const [, nextLabel] = DISPLAY_MODE_OPTIONS[(safeIndex + 1) % DISPLAY_MODE_OPTIONS.length] ?? DISPLAY_MODE_OPTIONS[0];
  const accessibleName = `Display mode: ${currentLabel}. Switch to ${nextLabel}`;
  return `<button type="button" id="display-mode-toggle" class="icon-button display-mode-button" data-action="display-mode" aria-label="${escapeHtml(accessibleName)}" title="${escapeHtml(accessibleName)}">${iconMarkup('display')}</button>`;
}

export function renderDeviceCardHeader({id, label, availabilityLabel, availabilityTone = '', availabilityStatus = 'unavailable', isSimulated = false, lastSeen = ''}: DeviceCardHeaderOptions): string {
  const escapedId = escapeHtml(id);
  const escapedLabel = escapeHtml(label);
  const kind = isSimulated ? 'SIMULATED BLIND' : 'YOUR BLIND';
  return `<div class="card-top device-card-header"><div class="device-card-heading"><div class="subtle">${kind}</div><h3><button type="button" class="card-title" data-action="details:${escapedId}" aria-label="View details for ${escapedLabel}">${escapedLabel}</button></h3><div class="subtle">${escapeHtml(lastSeen)}</div></div>${renderStatusIndicator({label:availabilityLabel, tone:availabilityTone, status:availabilityStatus})}</div>`;
}

export function renderButton({
  label,
  action,
  variant = '',
  disabled = false,
  icon = null,
  iconOnly = false,
  accessibleLabel = label,
  title = '',
}: ButtonOptions): string {
  const classes = ['button', variant, iconOnly ? 'icon-action' : ''].filter(Boolean).map(escapeHtml).join(' ');
  const accessibility = iconOnly || accessibleLabel !== label ? ` aria-label="${escapeHtml(accessibleLabel)}"` : '';
  const tooltipLabel = title || (iconOnly ? accessibleLabel : '');
  const titleAttribute = tooltipLabel
    ? ` title="${escapeHtml(tooltipLabel)}"`
    : '';
  const iconContent = iconMarkup(icon);
  const labelContent = iconOnly ? '' : `<span class="button-label">${escapeHtml(label)}</span>`;
  return `<button type="button" class="${classes}" data-action="${escapeHtml(action)}"${accessibility}${titleAttribute}${disabled ? ' disabled' : ''}>${iconContent}${labelContent}</button>`;
}

export function renderIconButton({label, action, icon, variant = '', disabled = false}: IconButtonOptions): string {
  return renderButton({
    label,
    accessibleLabel:label,
    title:label,
    action,
    variant:['icon-button', variant].filter(Boolean).join(' '),
    disabled,
    icon,
    iconOnly:true,
  });
}

export function renderDestructiveButton({label, action, disabled = false, icon = 'trash'}: {label: string; action: string; disabled?: boolean; icon?: string}): string {
  return renderButton({label, action, variant:'danger', disabled, icon});
}

export function renderMotorSpeedControl({deviceId, busy = false, calibrationActive = false}: {deviceId: string; busy?: boolean; calibrationActive?: boolean}): string {
  const id = escapeHtml(deviceId);
  const disabled = busy || calibrationActive;
  return `<section class="panel device-settings"><h2>Motor speed</h2><p>Set the motor speed from 0% to 100%. SOMA controllers need firmware 2.2.0 or later. The current setting is not read by Somaris yet.</p><form data-motor-speed="${id}"><label class="field">Speed (%)<input name="motor_speed_percent" type="number" min="0" max="100" step="1" required placeholder="0–100"${disabled ? ' disabled' : ''}></label><button class="button"${disabled ? ' disabled' : ''}>Set speed</button></form></section>`;
}

export function renderCalibrationControls({deviceId, active = false, busy = false}: {deviceId: string; active?: boolean; busy?: boolean}): string {
  const id = String(deviceId ?? '');
  if (!active) {
    return renderButton({
      label:'Start endpoint calibration',
      action:`command:calibration_start:${id}`,
      variant:'secondary',
      disabled:busy,
    });
  }
  const controls = [
    ['Nudge open', 'calibration_nudge_open'],
    ['Set open limit', 'calibration_set_open_limit'],
    ['Nudge closed', 'calibration_nudge_closed'],
    ['Set closed limit', 'calibration_set_closed_limit'],
    ['Stop movement', 'stop'],
    ['End calibration', 'calibration_stop'],
  ] as const;
  return `<div class="calibration-actions">${controls.map(([label, action]) => renderButton({
    label,
    action:`command:${action}:${id}`,
    variant:action === 'calibration_stop' ? 'danger' : 'secondary',
    disabled:busy,
  })).join('')}</div>`;
}

export function renderConfirmationPanel({
  title,
  message,
  confirmLabel = 'Confirm',
  confirmAction,
  cancelAction,
  disabled = false,
}: {title: string; message: string; confirmLabel?: string; confirmAction: string; cancelAction: string; disabled?: boolean}): string {
  return `<section class="confirmation-panel panel" role="group" aria-label="${escapeHtml(title)}"><div class="confirmation-copy"><h2>${escapeHtml(title)}</h2><p>${escapeHtml(message)}</p></div><div class="confirmation-actions">${renderButton({label:'Cancel', action:cancelAction, variant:'secondary', disabled})}${renderDestructiveButton({label:confirmLabel, action:confirmAction, disabled})}</div></section>`;
}

export function renderAddBlindButton({variant = ''}: {variant?: string} = {}): string {
  return renderIconButton({
    label:'Add a blind',
    action:'add',
    variant:['add-icon-button', 'topbar-add', variant].filter(Boolean).join(' '),
    icon:'add',
  });
}

export function renderTopbarActions({
  blindCount = 0,
  panelMode = false,
  displayPreference = 'auto',
  serviceLabel = 'Connecting',
  serviceTone = '',
}: TopbarActionsOptions = {}): string {
  const count = Number.isInteger(blindCount) && blindCount >= 0 ? blindCount : 0;
  const countLabel = `${count} ${count === 1 ? 'blind' : 'blinds'}`;
  const status = `<span id="service-status" class="service-status"><span class="status-dot${serviceTone ? ` ${escapeHtml(serviceTone)}` : ''}"></span> ${escapeHtml(serviceLabel)}</span>`;
  const panelCount = panelMode ? `<span class="topbar-device-count">${escapeHtml(countLabel)}</span>` : '';
  const displayToggle = renderDisplayModeToggle({displayPreference});
  const refreshButton = renderIconButton({label:'Refresh', action:'refresh', icon:'refresh', variant:'topbar-icon-button'});
  return `${status}${panelCount}${renderAddBlindButton()}${displayToggle}${refreshButton}`;
}

export function renderBlindActions({deviceId, blindLabel, panelMode = false, disabled = false, busy = false, movementDisabled = false, supportsPosition = false, positionClosedPercent = 100}: BlindActionsOptions): string {
  const actions = [
    ['open', 'Open', ''],
    ['stop', 'Stop', 'secondary'],
    ['close', 'Close', 'secondary'],
  ] as const;
  return `<div class="controls" role="group" aria-label="${escapeHtml(blindLabel)} controls">${actions.map(([action, label, variant]) => {
    const accessibleLabel = `${label} ${blindLabel}`;
    return renderButton({
      label,
      accessibleLabel,
      action:`command:${action}:${deviceId}`,
      variant,
      disabled:disabled || (movementDisabled && action !== 'stop') || (busy && action !== 'stop'),
      icon:action,
      iconOnly:panelMode,
      title:panelMode ? accessibleLabel : '',
    });
  }).join('')}${supportsPosition && !panelMode ? `<div class="position-control"><label for="position-${escapeHtml(deviceId)}">Target position <output data-position-output="${escapeHtml(deviceId)}">${escapeHtml(positionClosedPercent)}%</output> closed</label><input id="position-${escapeHtml(deviceId)}" type="range" min="0" max="100" step="1" value="${escapeHtml(positionClosedPercent)}" data-position-input="${escapeHtml(deviceId)}" aria-label="Target position for ${escapeHtml(blindLabel)} in percent closed"${disabled || movementDisabled || busy ? ' disabled' : ''}><button type="button" class="button secondary position-submit" data-action="position:${escapeHtml(deviceId)}"${disabled || movementDisabled || busy ? ' disabled' : ''}>Move to position</button></div>` : ''}</div>`;
}

const batchFailureStatuses = new Set(['connection_failed', 'deferred', 'write_failed', 'outcome_unknown', 'expired', 'cancelled']);
const batchSkipLabels: Record<string, string> = {
  not_seen:'not seen recently',
  stale:'stale observation',
  ambiguous:'ambiguous device',
  cooldown:'movement cooldown',
  calibration_active:'calibration active',
};

export function renderBatchFeedback(batch: BatchFeedbackView | null | undefined): string {
  if (!batch) return '';
  const data: BatchFeedbackView = batch.data || batch;
  const action = batch.action || data.action;
  const actionLabel = action === 'close' ? 'Close' : 'Open';
  const status = batch.status || data.status;
  if (status && ['completed', 'partial', 'failed', 'rejected'].includes(status)) return '';
  let message = '';
  let tone = '';

  if (status === 'submitting') message = `Sending ${actionLabel.toLowerCase()} commands to ready blinds…`;
  else if (status === 'checking') message = `Checking the ${actionLabel.toLowerCase()} batch…`;
  else if (status === 'outcome_unknown') {
    message = `The ${actionLabel.toLowerCase()} batch result is unknown. Check its status before starting another batch.`;
    tone = 'warn';
  } else if (status === 'queued') {
    message = `${actionLabel} batch queued for ${data.target_count || 0} ready blinds.`;
  } else if (status === 'running') {
    const acknowledged = data.acknowledged_count || 0;
    const total = data.target_count || 0;
    const failedCount = data.failed_count || 0;
    const pending = data.pending_count || 0;
    const movement = action === 'close' ? 'Closing' : 'Opening';
    const failed = failedCount ? `${failedCount} failed, ` : '';
    message = `${movement} blinds: commands acknowledged for ${acknowledged} of ${total} ready blinds; ${failed}${pending} pending.`;
  } else {
    return '';
  }

  const skippedItems = data.skipped || [];
  const skipped = skippedItems.length
    ? `<div class="batch-skipped"><span>Skipped blinds</span><ul>${skippedItems.map(item => `<li>${escapeHtml(item.label)} · ${escapeHtml(batchSkipLabels[item.availability ?? ''] || item.availability || 'unavailable')}</li>`).join('')}</ul></div>`
    : '';
  const failures = (data.targets || []).filter((target: {status: CommandStatus}) => batchFailureStatuses.has(target.status));
  const failed = failures.length
    ? `<div class="batch-failures"><span>Unsuccessful blinds</span><ul>${failures.map(item => `<li>${escapeHtml(item.label)} · ${escapeHtml(item.status.replaceAll('_', ' '))}</li>`).join('')}</ul></div>`
    : '';
  const retry = status === 'outcome_unknown'
    ? renderButton({label:batch.id ? 'Check batch status' : 'Retry original request', action:'batch:check', variant:'secondary'})
    : '';
  const errorDetail = status === 'outcome_unknown' && batch.error
    ? `<span class="subtle">${escapeHtml(batch.error)}</span>`
    : '';
  return `<div class="batch-feedback${tone ? ` batch-feedback--${tone}` : ''}" role="status" aria-live="polite"><p>${escapeHtml(message)}</p>${errorDetail}${skipped}${failed}${retry}</div>`;
}

export function renderSnackbar({id, message, tone = 'info', action = null, details = []}: SnackbarRenderOptions = {}): string {
  if (typeof message !== 'string' || !message.trim()) return '';
  const safeTone: string = ['info', 'success', 'warning', 'error'].includes(tone ?? '') ? tone ?? 'info' : 'info';
  const isError = safeTone === 'error';
  const actionMarkup = action && typeof action.label === 'string' && typeof action.action === 'string'
    ? `<button type="button" class="snackbar-action" data-snackbar-control="action" data-action="${escapeHtml(action.action)}">${escapeHtml(action.label)}</button>`
    : '';
  const detailItems=Array.isArray(details)&&details.length
    ? `<ul class="snackbar-details">${details.map(detail=>`<li>${escapeHtml(detail)}</li>`).join('')}</ul>`
    : '';
  return `<article class="snackbar snackbar--${safeTone}" data-snackbar-id="${escapeHtml(id || '')}" role="${isError ? 'alert' : 'status'}" aria-live="${isError ? 'assertive' : 'polite'}" aria-atomic="true"><span class="snackbar-message">${escapeHtml(message)}</span>${detailItems}${actionMarkup}<button type="button" class="snackbar-dismiss" data-snackbar-control="dismiss" data-action="snackbar-dismiss:${escapeHtml(id || '')}" aria-label="Dismiss notification" title="Dismiss">×</button></article>`;
}

export function renderSnackbarRegion(items: SnackbarRenderOptions[] = []): string {
  return `<div id="snackbar-region" class="snackbar-region" role="region" aria-label="Notifications">${(items || []).map(renderSnackbar).join('')}</div>`;
}

export function renderBatchControls({blindCount = 0, eligibleCount = 0, disabled = false, busy = false, panelMode = false, batch = null}: BatchControlsOptions = {}): string {
  if (blindCount < 1 && !batch) return '';
  const unavailable = disabled || busy || eligibleCount < 1;
  const description = eligibleCount === 1
    ? 'Open or close the available blind.'
    : eligibleCount > 1
      ? `Open or close all ${eligibleCount} available blinds in sequence.`
      : 'No enrolled blinds are currently ready to move.';
  const panelVariant = panelMode ? ' all-blinds-controls--panel' : '';
  const controls = blindCount > 0
    ? `<section class="all-blinds-controls${panelVariant}" aria-label="All blinds controls"${busy ? ' aria-busy="true"' : ''}><div class="all-blinds-copy"><h2>All blinds</h2><p>${escapeHtml(description)}</p></div><div class="all-blinds-actions">${renderButton({label:'Open all', accessibleLabel:'Open all blinds', action:'batch:open', icon:'open', disabled:unavailable})}${renderButton({label:'Close all', accessibleLabel:'Close all blinds', action:'batch:close', icon:'close', variant:'secondary', disabled:unavailable})}</div>${renderBatchFeedback(batch)}</section>`
    : '';
  return `${controls}${blindCount < 1 ? renderBatchFeedback(batch) : ''}`;
}

export function renderBlindCard({
  id,
  label,
  isSimulated,
  lastSeen,
  availabilityLabel,
  availabilityTone = '',
  availabilityStatus = 'unavailable',
  positionPercent,
  movementStatus,
  blindLines = 0,
  controlsMarkup,
  messageMarkup = '',
  footerMarkup = '',
  panelMode = false,
}: BlindCardOptions): string {
  const slats = '<i></i>'.repeat(Math.max(0, Math.min(11, blindLines)));
  const cardVariant = panelMode ? ' card--panel' : '';
  const displayVariant = panelMode ? ' blind-display--panel' : '';
  const header = renderDeviceCardHeader({id, label, availabilityLabel, availabilityTone, availabilityStatus, isSimulated, lastSeen});
  const status = renderBlindStatus({positionPercent, movementStatus, panelMode});
  return `<article class="card${cardVariant}">${header}<div class="blind-display${displayVariant}">${status}<div class="blind-art" aria-hidden="true">${slats}</div></div>${controlsMarkup}${messageMarkup}<div class="card-foot">${footerMarkup}</div></article>`;
}

export function renderDeviceList({count, panelMode = false, cardsMarkup = '', emptyMarkup = '', toolbarMarkup = '', title = 'Your blinds'}: DeviceListOptions): string {
  const titleContent = panelMode ? '' : `<h2>${escapeHtml(title)}</h2>`;
  const countLabel = panelMode ? '' : `<span class="count">${count} ${count === 1 ? 'blind' : 'blinds'}</span>`;
  const content = cardsMarkup ? `<div class="grid">${cardsMarkup}</div>` : emptyMarkup;
  const heading = panelMode ? '' : `<div class="section-heading device-list-heading">${titleContent}${countLabel}</div>`;
  return `<section class="device-list${panelMode ? ' device-list--panel' : ''}" aria-label="${escapeHtml(title)}">${heading}${toolbarMarkup}${content}</section>`;
}

export function renderHomeSupportingSections({panelMode = false, activityMarkup = '', serviceMarkup = ''}: {panelMode?: boolean; activityMarkup?: string; serviceMarkup?: string}): string {
  return panelMode ? '' : `${activityMarkup}${serviceMarkup}`;
}

export function renderInfoItem({label, value, contentMarkup}: InfoItemOptions): string {
  const content = contentMarkup === undefined ? `<strong>${escapeHtml(value)}</strong>` : contentMarkup;
  return `<div class="info-item"><span class="meta-label subtle">${escapeHtml(label)}</span>${content}</div>`;
}

export function renderInfoGrid(items: InfoItemOptions[]): string {
  return `<div class="info-grid">${items.map(renderInfoItem).join('')}</div>`;
}
