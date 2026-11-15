import { escapeHtml, renderBlindActions, renderBlindCard, renderBlindStatus, renderButton, renderDestructiveButton, renderConfirmationPanel, renderDeviceList, renderTopbarActions, DISPLAY_MODE_OPTIONS, renderHomeSupportingSections, renderBatchControls, renderPanelNotice, renderInfoGrid, renderMotorSpeedControl, renderCalibrationControls, movementStatusFromPositions, renderSnackbar, renderSnackbarRegion, } from './components.mjs';
import { SnackbarQueue } from './snackbar.mjs';
export { escapeHtml };
const displayModes = new Set(DISPLAY_MODE_OPTIONS.map(([value]) => value));
const displayPreferenceCycle = DISPLAY_MODE_OPTIONS.map(([value]) => value);
export function nextDisplayPreference(preference) {
    const index = displayPreferenceCycle.indexOf(preference);
    return displayPreferenceCycle[(index + 1 + displayPreferenceCycle.length) % displayPreferenceCycle.length] ?? 'auto';
}
export function resolveDisplayMode(preference, width, height) {
    if (displayModes.has(preference) && preference !== 'auto')
        return preference;
    const viewportWidth = Number.isFinite(width) ? Math.max(0, width) : 0;
    const viewportHeight = Number.isFinite(height) ? Math.max(0, height) : 0;
    const aspectRatio = viewportHeight ? viewportWidth / viewportHeight : 0;
    if (viewportWidth > 0 && viewportHeight > 0 && Math.max(viewportWidth, viewportHeight) <= 1100 && aspectRatio >= 0.82 && aspectRatio <= 1.22)
        return 'panel';
    return viewportWidth <= 600 ? 'mobile' : 'desktop';
}
export function displayPreferenceFrom(search, saved) {
    const requested = new URLSearchParams(search || '').get('view');
    if (requested && displayModes.has(requested))
        return requested;
    return saved && displayModes.has(saved) ? saved : 'auto';
}
export function resolveActionFocus({ focusedAction = null, pendingConfirmationId = null, confirmingRemovalId = null, availableActions = [], }) {
    if (pendingConfirmationId && pendingConfirmationId === confirmingRemovalId) {
        const cancelAction = `remove-cancel:${encodeURIComponent(pendingConfirmationId)}`;
        if (availableActions.includes(cancelAction))
            return cancelAction;
    }
    if (focusedAction && availableActions.includes(focusedAction))
        return focusedAction;
    if (focusedAction?.startsWith('remove-cancel:')) {
        const removeAction = `remove:${focusedAction.slice('remove-cancel:'.length)}`;
        if (availableActions.includes(removeAction))
            return removeAction;
    }
    return null;
}
function storedDisplayPreference() {
    let saved;
    try {
        saved = globalThis.localStorage?.getItem('somaris-display-mode');
    }
    catch { /* Storage can be unavailable in private browsing contexts. */ }
    return displayPreferenceFrom(globalThis.location?.search, saved);
}
export function commandMessage(command) {
    switch (command.status) {
        case 'queued': return ['Queued. Waiting for the blind…', ''];
        case 'connecting': return ['Sending command to the blind…', ''];
        case 'deferred': return ['Movement paused by the Bluetooth cooldown. Stop remains available.', 'warn'];
        case 'write_acknowledged': return ['Command acknowledged. Movement is shown when reported by the blind.', ''];
        case 'outcome_unknown':
        case 'write_failed': return ['The outcome is uncertain. Check the reported position before trying again.', 'warn'];
        case 'connection_failed':
            if (command.failure_stage === 'command_validation')
                return ['Command data is missing or invalid. No Bluetooth write was attempted.', 'bad'];
            return [command.retryable ? 'Could not connect. No command was sent; you can try again.' : 'Could not connect to the blind.', 'bad'];
        case 'expired': return ['The command expired before it could be sent.', 'bad'];
        case 'cancelled': return ['The command was cancelled.', 'warn'];
        default: return ['Checking command status…', ''];
    }
}
export function activityMessage(activity) {
    const subject = activity.subject || 'Somaris';
    const action = activity.action ? `${activity.action.charAt(0).toUpperCase()}${activity.action.slice(1)}` : '';
    const status = activity.status?.replaceAll('_', ' ');
    switch (activity.kind) {
        case 'enrollment_added': return `Added ${subject}`;
        case 'enrollment_removed': return `Removed ${subject}`;
        case 'command': return `${action} command${status ? ` · ${status}` : ''} · ${subject}`;
        case 'batch': return `${action} batch${status ? ` · ${status}` : ''} · ${subject}`;
        case 'simulator_settings_changed': return `Updated simulator settings · ${subject}${activity.detail ? ` · ${activity.detail}` : ''}`;
        case 'simulator_reset': return `Reset simulator · ${subject}${activity.detail ? ` · ${activity.detail}` : ''}`;
        default: return `Changed ${subject}`;
    }
}
export function observationFor(managed, observations) {
    const e = managed.enrollment;
    const matches = observations.filter(o => o.logical_id === e.logical_id && o.advertised_name === e.advertised_name && !!o.is_simulated === !!e.is_simulated);
    return matches.length === 1 ? matches[0] ?? null : null;
}
export function batteryDisplay(battery) {
    const percentage = battery?.percent_estimate;
    if (!battery || typeof percentage !== 'number' || !Number.isInteger(percentage) || percentage < 0 || percentage > 100)
        return '<span class="subtle">Battery unavailable</span>';
    const cached = battery.status !== 'observed' ? ' · cached' : '';
    const voltageMv = battery.voltage_mv;
    const voltage = typeof voltageMv === 'number' && Number.isInteger(voltageMv) && voltageMv > 0 ? `Battery voltage: ${voltageMv} mV. ` : '';
    return `<div class="battery-reading subtle">Battery ~${percentage}%${cached}<details class="battery-info"><summary aria-label="Battery information">i</summary><span class="battery-tooltip">${voltage}Percentage is an estimate based on voltage.</span></details></div>`;
}
const terminal = new Set(['write_acknowledged', 'connection_failed', 'write_failed', 'outcome_unknown', 'expired', 'cancelled', 'deferred']);
const app = typeof document === 'undefined' ? null : document.querySelector('#app');
const snackbarRoot = typeof document === 'undefined' ? null : document.querySelector('#snackbar-root');
let renderSnackbars = () => { };
const snackbarQueue = new SnackbarQueue({ onChange: items => renderSnackbars(items) });
const state = { demoMode: false, managed: [], observations: [], diagnostics: null, activity: [], commands: {}, batch: null, serviceError: null, confirmingRemovalId: null, pendingRemovalFocusId: null, removingEnrollmentId: null, busy: new Set(), reconciliationRequired: new Set(), tokens: {}, simulators: null, notify: item => snackbarQueue.push(item), dismissNotification: key => snackbarQueue.dismissByKey(key) };
let demoSessionRecovery = null;
let displayPreference = storedDisplayPreference();
const esc = escapeHtml;
function relativeAt(date, now) {
    if (!date)
        return 'Never seen';
    const seconds = Math.max(0, Math.floor((now - new Date(date).getTime()) / 1000));
    if (!Number.isFinite(seconds))
        return 'Time unavailable';
    if (seconds < 60)
        return 'Seen just now';
    if (seconds < 3600)
        return `Seen ${Math.floor(seconds / 60)} min ago`;
    if (seconds < 86400)
        return `Seen ${Math.floor(seconds / 3600)} hr ago`;
    return `Seen ${Math.floor(seconds / 86400)} days ago`;
}
const relative = (date) => relativeAt(date, Date.now());
function isRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function apiError(error) {
    if (error instanceof Error)
        return error;
    return new Error(String(error));
}
function errorMessage(error) {
    return error instanceof Error ? error.message : String(error);
}
async function api(path, options = {}, recoverExpiredSession = true) {
    const response = await fetch(path, { cache: 'no-store', credentials: 'same-origin', ...options });
    const body = await response.json().catch(() => ({}));
    const envelope = isRecord(body) ? body : {};
    const errorValue = isRecord(envelope.error) ? envelope.error : {};
    if (!response.ok) {
        const error = new Error(String(errorValue.message ?? envelope.message ?? `Request failed (${response.status})`));
        error.status = response.status;
        if (typeof errorValue.code === 'string')
            error.code = errorValue.code;
        if (recoverExpiredSession && state.demoMode && error.code === 'demo_session_expired' && path !== '/api/demo/session') {
            if (!demoSessionRecovery) {
                demoSessionRecovery = api('/api/demo/session', { method: 'POST' }, false)
                    .then(() => {
                    state.commands = {};
                    state.batch = null;
                    state.busy.clear();
                    state.reconciliationRequired.clear();
                    snackbarQueue.push({ message: 'Your demo session expired. A fresh demonstration was started.', tone: 'warning' });
                })
                    .finally(() => { demoSessionRecovery = null; });
            }
            await demoSessionRecovery;
            if (!options.method || options.method.toUpperCase() === 'GET')
                return api(path, options, false);
            error.message = 'Your demo session expired and a fresh demo was started. Review the reset demo before continuing.';
        }
        throw error;
    }
    return envelope.data;
}
function button(label, action, extra = '', disabled = false, icon = null) {
    return renderButton({ label, action, variant: extra, disabled, icon });
}
function notifyTarget(target, notification) {
    if (typeof target?.notify === 'function')
        target.notify(notification);
    else {
        target.notifications ??= [];
        target.notifications.push(notification);
    }
}
function dismissTargetNotification(target, key) { target.dismissNotification?.(key); }
function serviceFailure(target, error) {
    const message = `Could not refresh Somaris: ${errorMessage(error)}`;
    if (target.serviceError !== message) {
        if (target.serviceError)
            dismissTargetNotification(target, 'service-refresh');
        notifyTarget(target, { key: 'service-refresh', message, tone: 'error' });
    }
    target.serviceError = message;
}
function serviceRecovered(target) {
    const recovered = !!target.serviceError;
    target.serviceError = null;
    if (recovered) {
        dismissTargetNotification(target, 'service-refresh');
        notifyTarget(target, { key: 'service-restored', message: 'The connection to Somaris is back.', tone: 'success' });
    }
}
renderSnackbars = (items) => {
    if (!snackbarRoot)
        return;
    const focused = snackbarRoot.contains(document.activeElement) ? document.activeElement : null;
    const focusedId = focused?.closest('[data-snackbar-id]')?.dataset.snackbarId;
    const focusedControl = focused?.dataset.snackbarControl;
    if (!snackbarRoot.querySelector('#snackbar-region'))
        snackbarRoot.innerHTML = renderSnackbarRegion([]);
    const region = snackbarRoot.querySelector('#snackbar-region');
    if (!region)
        return;
    const visibleIds = new Set(items.map(item => item.id));
    for (const item of [...region.querySelectorAll('[data-snackbar-id]')]) {
        if (!visibleIds.has(item.dataset.snackbarId ?? ''))
            item.remove();
    }
    for (const item of items) {
        if (region.querySelector(`[data-snackbar-id="${CSS.escape(item.id)}"]`))
            continue;
        const template = document.createElement('template');
        template.innerHTML = renderSnackbar(item);
        const snackbar = template.content.firstElementChild;
        if (snackbar)
            region.append(snackbar);
    }
    if (focusedId && focusedControl) {
        const itemSelector = `[data-snackbar-id="${CSS.escape(focusedId)}"] [data-snackbar-control="${CSS.escape(focusedControl)}"]`;
        snackbarRoot.querySelector(itemSelector)?.focus({ preventScroll: true });
    }
};
if (snackbarRoot)
    renderSnackbars(snackbarQueue.visibleItems);
const availabilityLabels = {
    ready: ['Observed recently', ''], stale: ['Stale', 'warn'], not_seen: ['Not seen', 'warn'],
    ambiguous: ['Ambiguous', 'bad'], unavailable: ['Unavailable', 'warn'],
};
export function availability(value) {
    return value ? availabilityLabels[value] : availabilityLabels.unavailable;
}
const terminalBatchStatuses = new Set(['completed', 'partial', 'failed', 'rejected']);
function blindBatchIsLocked(batch) {
    return !!batch && !terminalBatchStatuses.has(batch.status);
}
export function batchEligibleCount(managed, now = Date.now()) {
    return (managed || []).filter(device => {
        if (device.availability !== 'ready' || device.calibration_mode_active === true)
            return false;
        const cooldownUntil = Date.parse(device.gatt_readiness?.cooldown_until || '');
        return !Number.isFinite(cooldownUntil) || cooldownUntil <= now;
    }).length;
}
function newIdempotencyKey() {
    return globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}
function batchRequestOptions(batch) {
    return {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'Idempotency-Key': batch.idempotencyKey ?? '' },
        body: JSON.stringify({ action: batch.action }),
    };
}
const batchFailureStatuses = new Set(['connection_failed', 'deferred', 'write_failed', 'outcome_unknown', 'expired', 'cancelled']);
function batchNotificationCopy(label, status, acknowledged, total, failed, skipped, error) {
    const unavailable = skipped ? `; ${skipped} unavailable` : '';
    switch (status) {
        case 'completed':
            return {
                message: `${label} commands acknowledged for ${acknowledged} of ${total} ready blinds${unavailable}. Acknowledgement does not confirm movement.`,
                tone: skipped ? 'warning' : 'success',
            };
        case 'partial':
            return { message: `${label} commands acknowledged for ${acknowledged} of ${total} ready blinds; ${failed} failed${unavailable}.`, tone: 'warning' };
        case 'failed':
            return { message: `No ${label.toLowerCase()} commands were acknowledged; ${failed} failed${unavailable}.`, tone: 'error' };
        case 'rejected':
            return { message: `Could not start the ${label.toLowerCase()} batch: ${error || 'The request was rejected.'}`, tone: 'error' };
        default:
            return null;
    }
}
function batchNotificationDetails(data) {
    const skipped = (data.skipped || []).map(item => `Skipped ${item.label || 'blind'} · ${(item.availability || 'unavailable').replaceAll('_', ' ')}`);
    const failures = (data.targets || [])
        .filter(target => batchFailureStatuses.has(target.status))
        .map(target => `${target.label || 'Blind'} · ${target.status.replaceAll('_', ' ')}`);
    return [...skipped, ...failures];
}
function batchResultNotification(batch) {
    const data = batch.data || batch;
    const label = (batch.action || data.action) === 'close' ? 'Close' : 'Open';
    const status = batch.status || data.status;
    const skipped = data.skipped?.length ?? 0;
    const copy = batchNotificationCopy(label, status, data.acknowledged_count || 0, data.target_count || 0, data.failed_count || 0, skipped, batch.error || data.error);
    if (!copy)
        return null;
    const details = batchNotificationDetails(data);
    const visibleDetails = details.slice(0, 4);
    if (details.length > visibleDetails.length)
        visibleDetails.push(`${details.length - visibleDetails.length} more details`);
    return { key: `batch:${batch.id || batch.idempotencyKey || 'current'}:result`, ...copy, details: visibleDetails };
}
function applyBatchResult(target, batch, result, onChange) {
    if (target.batch !== batch)
        return false;
    batch.id = result.id || batch.id;
    batch.data = result;
    batch.status = result.status;
    batch.error = result.error || null;
    const notification = batchResultNotification(batch);
    if (notification) {
        dismissTargetNotification(target, `batch:${batch.id || batch.idempotencyKey || 'current'}:unknown`);
        notifyTarget(target, notification);
    }
    onChange();
    return true;
}
export async function submitBlindBatch(target, request, action, onChange = () => { }, createKey = newIdempotencyKey) {
    if (!['open', 'close'].includes(action) || blindBatchIsLocked(target.batch))
        return false;
    const batch = { action, status: 'submitting', idempotencyKey: createKey(), id: null, data: null, error: null };
    target.batch = batch;
    onChange();
    try {
        const result = await request('/api/control/v1/batches', batchRequestOptions(batch));
        return applyBatchResult(target, batch, result, onChange);
    }
    catch (error) {
        if (target.batch !== batch)
            return false;
        const failure = apiError(error);
        batch.error = String(failure.message || error);
        const statusCode = failure.status ?? 0;
        batch.status = statusCode >= 400 && statusCode < 500 ? 'rejected' : 'outcome_unknown';
        if (batch.status === 'rejected') {
            const notification = batchResultNotification(batch);
            if (notification)
                notifyTarget(target, notification);
        }
        else {
            notifyTarget(target, { key: `batch:${batch.id || batch.idempotencyKey}:unknown`, message: `The ${action} batch result is unknown. Check its status before starting another batch.`, tone: 'warning', action: { label: 'Check status', action: 'batch:check' } });
        }
        onChange();
        return false;
    }
}
export async function checkBlindBatch(target, request, onChange = () => { }) {
    const batch = target.batch;
    if (!batch || terminalBatchStatuses.has(batch.status))
        return false;
    const previousStatus = batch.status;
    batch.status = 'checking';
    batch.error = null;
    onChange();
    try {
        const result = batch.id
            ? await request(`/api/control/v1/batches/${encodeURIComponent(batch.id)}`)
            : await request('/api/control/v1/batches', batchRequestOptions(batch));
        return applyBatchResult(target, batch, result, onChange);
    }
    catch (error) {
        if (target.batch !== batch)
            return false;
        batch.status = 'outcome_unknown';
        batch.error = errorMessage(error);
        if (previousStatus !== 'outcome_unknown')
            notifyTarget(target, { key: `batch:${batch.id || batch.idempotencyKey}:unknown`, message: `The ${batch.action} batch result is unknown. Check its status before starting another batch.`, tone: 'warning', action: { label: 'Check status', action: 'batch:check' } });
        onChange();
        return false;
    }
}
export function gattReadinessLabel(readiness) {
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
export function gattTransportEvidenceLabel(evidence) {
    const labels = {
        untested: 'No transport evidence yet',
        command_pending: 'A command is in progress',
        recently_acknowledged: 'A GATT write was recently acknowledged',
        acknowledgement_aged: 'The last GATT acknowledgement is old',
        prewrite_failure: 'The latest command failed before a write',
        write_failed: 'The latest write failed; delivery may be uncertain',
        outcome_unknown: 'The latest write outcome is unknown',
        no_recent_write: 'No recent write evidence',
        cooldown: 'Movement command cooldown is active',
    };
    return (evidence ? labels[evidence] : undefined) || 'Transport evidence unavailable';
}
export function motionFeedbackLabel(feedback) {
    const labels = {
        awaiting_progress: 'Waiting for position progress',
        progress_observed: 'Position progress observed',
        stalled: 'No recent position progress',
        target_reached: 'Motor reports the target position',
        unavailable: 'Position feedback unavailable',
    };
    const status = feedback?.status;
    return (status ? labels[status] : undefined) || 'No movement feedback';
}
function dateTimeLabel(value, fallback) {
    if (!value)
        return fallback;
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? 'Time unavailable' : date.toLocaleString();
}
function cardCommandMessage(command) {
    if (!command)
        return ['', ''];
    const contextual = !terminal.has(command.status)
        || ['outcome_unknown', 'write_failed', 'deferred'].includes(command.status);
    return contextual ? commandMessage(command) : ['', ''];
}
function cardAvailabilityDetail(device) {
    return device.availability === 'ready'
        ? ''
        : 'Controls are unavailable until this blind is uniquely observed again.';
}
function cardCooldownDetail(cooldownUntil, now) {
    if (!cooldownUntil || !(new Date(cooldownUntil).getTime() > now))
        return '';
    return `Repeated Bluetooth connection failures. Open and Close are paused until ${dateTimeLabel(cooldownUntil, 'the cooldown ends')}; Stop remains available.`;
}
function cardCalibrationDetail(device) {
    return device.calibration_mode_active
        ? 'Endpoint calibration is active. Normal movement is paused until calibration ends.'
        : '';
}
export function cardMessageMarkup(panelMode, message, tone, availabilityDetail, cooldownDetail, calibrationDetail) {
    if (!panelMode)
        return regularCardMessageMarkup(message, tone, availabilityDetail, cooldownDetail, calibrationDetail);
    const details = [message, availabilityDetail, cooldownDetail, calibrationDetail].filter(Boolean);
    if (!details.length)
        return '';
    return renderPanelNotice({
        label: panelCardNoticeLabel(message, availabilityDetail, cooldownDetail, calibrationDetail),
        details,
        tone: panelCardNoticeTone(tone, availabilityDetail, cooldownDetail, calibrationDetail),
    });
}
function panelCardNoticeLabel(message, availability, cooldown, calibration) {
    if (cooldown)
        return 'Movement paused';
    if (calibration)
        return 'Calibration active';
    if (availability)
        return 'Blind unavailable';
    return message ? 'Command update' : '';
}
function panelCardNoticeTone(tone, availability, cooldown, calibration) {
    if (tone)
        return tone;
    return availability || cooldown || calibration ? 'warn' : 'good';
}
function regularCardMessageMarkup(message, tone, availability, cooldown, calibration) {
    return [
        message ? `<p class="message ${tone}" role="status">${esc(message)}</p>` : '',
        availability ? `<p class="message warn">${esc(availability)}</p>` : '',
        cooldown ? `<p class="message warn">${esc(cooldown)}</p>` : '',
        calibration ? `<p class="message warn">${esc(calibration)}</p>` : '',
    ].join('');
}
export function renderManagedDeviceCard(device, dashboard, panelMode, now = Date.now()) {
    const e = device.enrollment, o = observationFor(device, dashboard.observations), [label, tone] = availability(device.availability), p = device.position?.current_closed_percent;
    const blindLabel = e.label ?? 'Blind';
    const cooldownUntil = device.gatt_readiness?.cooldown_until;
    const cooldownDetail = cardCooldownDetail(cooldownUntil, now);
    const [message, msgTone] = cardCommandMessage(dashboard.commands[e.id]);
    const targetPercent = device.position?.target_closed_percent ?? p ?? 100;
    const lines = typeof p === 'number' && Number.isInteger(p) ? Math.max(1, Math.round(p / 100 * 11)) : 0;
    const disabled = !!dashboard.serviceError || device.availability !== 'ready';
    const availabilityDetail = cardAvailabilityDetail(device);
    const calibrationDetail = cardCalibrationDetail(device);
    const controlsMarkup = renderBlindActions({
        deviceId: e.id, blindLabel,
        panelMode,
        disabled, busy: dashboard.busy.has(e.id),
        movementDisabled: !!cooldownDetail || device.calibration_mode_active === true || blindBatchIsLocked(dashboard.batch),
        supportsPosition: device.capabilities?.set_position === true,
        positionClosedPercent: targetPercent,
    });
    const messageMarkup = cardMessageMarkup(panelMode, message, msgTone, availabilityDetail, cooldownDetail, calibrationDetail);
    const footerMarkup = `${batteryDisplay(o?.battery)}<button type="button" class="text-button" data-action="details:${esc(e.id)}">View details &nbsp; ↗</button>`;
    return renderBlindCard({
        id: e.id, label: blindLabel, isSimulated: e.is_simulated, room: e.room, lastSeen: relativeAt(device.last_seen_at, now),
        availabilityLabel: label, availabilityTone: tone,
        availabilityStatus: device.availability,
        positionPercent: p, movementStatus: o?.movement_status ?? 'unavailable', blindLines: lines,
        controlsMarkup, messageMarkup, footerMarkup, panelMode,
    });
}
function card(device) {
    return renderManagedDeviceCard(device, state, document.documentElement.dataset.displayMode === 'panel');
}
function activityPanel() {
    const entries = state.activity || [];
    const list = entries.length
        ? `<ol class="activity-list">${entries.map(entry => {
            const occurredAt = entry.completed_at || entry.occurred_at;
            const occurredLabel = occurredAt ? new Date(occurredAt).toLocaleString() : 'Time unavailable';
            return `<li class="activity-item"><span class="activity-copy">${esc(activityMessage(entry))}${entry.detail && entry.kind !== 'simulator_settings_changed' && entry.kind !== 'simulator_reset' ? `<span class="subtle">${esc(entry.detail)}</span>` : ''}</span><time datetime="${esc(occurredAt ?? '')}">${esc(occurredLabel)}</time></li>`;
        }).join('')}</ol>`
        : '<div class="panel activity-empty"><p>No changes recorded yet.</p></div>';
    return `<section class="activity-section"><div class="section-heading"><h2>Recent activity</h2><span class="count">${entries.length} of 30 changes</span></div>${list}</section>`;
}
function home() {
    const panelMode = document.documentElement.dataset.displayMode === 'panel';
    const hero = panelMode ? '' : `<section class="hero"><div><p class="eyebrow">A little more light</p><h1>${state.demoMode ? 'Explore a simulated home.' : 'Your space, your way.'}</h1><p>${state.demoMode ? 'Control four simulated blinds. Your changes only affect this demonstration.' : 'A quieter way to keep your blinds just where you like them.'}</p></div></section>`;
    const empty = panelMode
        ? `<div class="panel empty"><h3>No blinds yet</h3><p>Add a nearby blind to see its status and controls here.</p></div>`
        : `<div class="panel empty"><div class="empty-icon">☼</div><h3>Let’s bring your blinds in.</h3><p>When a blind is nearby and discovered, add it here to see its status and control it.</p></div>`;
    const batchControls = renderBatchControls({
        blindCount: state.managed.length,
        eligibleCount: batchEligibleCount(state.managed),
        disabled: !!state.serviceError || state.busy.size > 0,
        busy: blindBatchIsLocked(state.batch),
        panelMode,
        batch: state.batch,
    });
    const listMarkup = renderDeviceList({
        count: state.managed.length, panelMode,
        cardsMarkup: state.managed.map(card).join(''), emptyMarkup: empty, toolbarMarkup: batchControls,
    });
    const serviceMarkup = `<div class="section-heading" style="margin-top:35px"><h2>Somaris service</h2><button type="button" class="text-button" data-action="service">View service status &nbsp; ↗</button></div><p class="subtle">${esc(scannerSummary())}</p>`;
    const supportingSections = renderHomeSupportingSections({
        panelMode,
        activityMarkup: panelMode ? '' : activityPanel(),
        serviceMarkup: panelMode ? '' : serviceMarkup,
    });
    return `${hero}${listMarkup}${supportingSections}`;
}
function scannerSummary() { const s = state.diagnostics?.scanner; if (state.demoMode)
    return state.diagnostics?.simulator_ready ? 'Demo ready · Simulated devices' : 'Demo simulator unavailable'; if (!s)
    return 'Service status unavailable'; return s.adapter_available ? (s.scan_active ? 'Bluetooth adapter available · scanning' : 'Bluetooth adapter available') : 'Bluetooth adapter unavailable · retrying'; }
function back() { return '<button class="text-button back" data-action="home">← &nbsp; Back to your blinds</button>'; }
function addPage() { const available = state.observations.filter(o => !state.managed.some(m => m.enrollment.logical_id === o.logical_id && !!m.enrollment.is_simulated === !!o.is_simulated)); return `${back()}<h1 class="page-title">Add a blind</h1><p class="page-intro">Choose a recently discovered blind to make it available for control.</p><div class="section-heading"><h2>Nearby devices</h2><span class="count">${available.length} found</span></div>${available.length ? `<div class="stack">${available.map(o => `<form class="panel device-row" data-enroll="${esc(o.id)}"><div><h3>${esc(o.advertised_name || o.device_name || 'Unnamed blind')}</h3><p class="subtle">${o.is_simulated ? 'Simulator · ' : ''}${esc(relative(o.last_seen_at))} · ${esc(o.freshness)}</p><label class="field">Display name<input name="label" maxlength="64" required value="${esc(o.advertised_name || o.device_name || 'Blind')}"></label></div><button class="button" ${o.freshness !== 'fresh' ? 'disabled' : ''}>Add blind</button></form>`).join('')}</div>` : `<div class="panel empty"><div class="empty-icon">⌁</div><h3>No new blinds found</h3><p>${state.diagnostics?.scanner?.adapter_available ? 'Keep your blind nearby and refresh in a moment.' : 'The Bluetooth adapter is unavailable. Check the service status and try again.'}</p>${button('Refresh devices', 'refresh', 'secondary')}</div>`}`; }
export function deviceDetailFields(device, observation, now = Date.now()) {
    const fields = [
        { label: 'Availability', value: availability(device.availability)[0] },
        { label: 'Position', contentMarkup: renderBlindStatus({ positionPercent: device.position?.current_closed_percent, movementStatus: observation?.movement_status ?? 'unavailable', compact: true }) },
        { label: 'Last observation', value: device.last_seen_at ? new Date(device.last_seen_at).toLocaleString() : 'Never' },
        { label: 'Advertisement age', value: typeof device.advertisement_age_ms === 'number' && Number.isFinite(device.advertisement_age_ms) ? `${Math.floor(device.advertisement_age_ms / 1000)} seconds` : 'Unavailable' },
        { label: 'Reported target', value: device.position ? `${device.position.target_closed_percent}% closed` : 'Unavailable' },
        { label: 'Movement feedback', value: motionFeedbackLabel(device.motion_feedback) },
    ];
    const travelRate = device.motion_feedback?.estimated_travel_rate_percent_per_second;
    if (typeof travelRate === 'number' && Number.isFinite(travelRate)) {
        fields.push({ label: 'Learned travel rate', value: `${travelRate.toFixed(2)}% per second` });
    }
    fields.push({ label: 'Signal', value: observation?.rssi_dbm != null ? `${observation.rssi_dbm} dBm` : 'Unavailable' }, { label: 'Battery voltage', value: observation?.battery?.voltage_mv != null ? `${observation.battery.voltage_mv} mV${observation.battery.status !== 'observed' ? ' (cached)' : ''}` : 'Unavailable' }, { label: 'Source', value: device.enrollment.is_simulated ? 'Simulator' : 'Bluetooth' });
    if (device.enrollment.room)
        fields.push({ label: 'Room', value: device.enrollment.room });
    if (device.gatt_readiness)
        fields.push(...gattDetailFields(device.gatt_readiness, now));
    return fields;
}
function gattDetailFields(readiness, now) {
    const fields = [
        { label: 'Last GATT command', value: gattReadinessLabel(readiness) },
        { label: 'Transport evidence', value: gattTransportEvidenceLabel(readiness.transport_evidence) },
        { label: 'Last command attempt', value: dateTimeLabel(readiness.last_attempt_at, 'Not attempted yet') },
        { label: 'Last acknowledged GATT write', value: dateTimeLabel(readiness.last_write_acknowledged_at, 'None recorded') },
    ];
    if (readiness.cooldown_until && new Date(readiness.cooldown_until).getTime() > now) {
        fields.push({ label: 'Movement cooldown until', value: dateTimeLabel(readiness.cooldown_until, 'None') });
    }
    fields.push({ label: 'Latest failure', value: readiness.error_code ? `${readiness.error_code} · ${readiness.failure_stage || 'unknown phase'}` : 'None recorded' });
    return fields;
}
function enrollmentConfirmation(id, device) {
    const encodedId = encodeURIComponent(id);
    if (state.confirmingRemovalId !== id) {
        return renderDestructiveButton({ label: 'Remove blind', action: `remove:${encodedId}` });
    }
    return renderConfirmationPanel({
        title: `Remove ${device.enrollment.label ?? 'Blind'}?`,
        message: 'This removes the enrollment from Somaris. It does not reset or change the physical blind, and the blind can be added again when it is nearby. Removal is blocked while a command is queued or running.',
        confirmLabel: 'Remove blind',
        confirmAction: `remove-confirm:${encodedId}`,
        cancelAction: `remove-cancel:${encodedId}`,
        disabled: state.removingEnrollmentId === id,
    });
}
function calibrationSection(id, device, busy) {
    if (!device.capabilities?.calibrate_endpoints)
        return '';
    const warning = device.calibration_mode_active
        ? '<p class="message warn" role="status">Calibration mode is active. Watch the blind while nudging it, then mark each endpoint.</p>'
        : '';
    return `<section class="panel device-settings"><h2>Endpoint calibration</h2><p>Calibration changes the blind’s stored open and closed limits. Stay with the blind while adjusting it. Use the small nudges to reach each endpoint, mark both limits, then end calibration.</p>${warning}${renderCalibrationControls({ deviceId: id, active: device.calibration_mode_active === true, busy })}</section>`;
}
function detailPage(id) {
    const device = state.managed.find(item => item.enrollment.id === id);
    if (!device)
        return `${back()}<div class="notice">This blind is no longer enrolled.</div>`;
    const observation = observationFor(device, state.observations);
    const busy = state.busy.has(id);
    const fields = renderInfoGrid(deviceDetailFields(device, observation));
    const motorSettings = device.capabilities?.set_motor_speed
        ? renderMotorSpeedControl({ deviceId: id, busy, calibrationActive: device.calibration_mode_active === true })
        : '';
    const calibration = calibrationSection(id, device, busy);
    const enrollment = enrollmentConfirmation(id, device);
    return `${back()}<h1 class="page-title">${esc(device.enrollment.label ?? 'Blind')}</h1><p class="page-intro">The latest observations reported by your blind.</p>${fields}<p class="message">Position is observed telemetry and may not reflect the current physical position.</p>${motorSettings}${calibration}<section class="panel enrollment-actions"><h2>Manage enrollment</h2><p>Remove this blind from your Somaris dashboard. If it is still nearby, it will remain discoverable and can be added again.</p>${enrollment}</section>`;
}
function simulatorControls() {
    if (!state.simulators?.length)
        return '';
    const options = (values, current) => values.map(([value, label]) => `<option value="${value}" ${value === current ? 'selected' : ''}>${label}</option>`).join('');
    const scenarios = [['normal', 'Normal'], ['pre_write_failure', 'Connection failure'], ['outcome_unknown', 'Unknown outcome'], ['gatt_write_error', 'Write error'], ['telemetry_paused', 'Telemetry paused']];
    const profiles = [['standard', 'Standard'], ['hallway_observed', 'Hallway observed'], ['hallway_slow_link', 'Hallway slow link']];
    const batteries = [['unsupported', 'Unsupported'], ['unavailable', 'Unavailable'], ['observed_sample', 'Observed sample']];
    const instances = state.simulators.map(simulator => {
        const movementStatus = movementStatusFromPositions(simulator.current_closed_percent, simulator.target_closed_percent);
        const position = renderBlindStatus({ positionPercent: simulator.current_closed_percent, movementStatus, compact: true });
        return `<div class="simulator-controls"><h3>${esc(simulator.advertised_name)}</h3><div class="simulator-position">${position}<span class="subtle">Simulator ID ${esc(simulator.id)}</span></div><form data-simulator="${esc(simulator.id)}"><label class="field">Scenario<select name="scenario">${options(scenarios, simulator.scenario ?? '')}</select></label><label class="field">Wireless profile<select name="wireless_profile">${options(profiles, simulator.wireless_profile ?? '')}</select></label><label class="field">Battery state<select name="battery_state">${options(batteries, simulator.battery_state ?? '')}</select></label><button class="button">Apply settings</button></form><button class="button secondary" style="margin-top:12px" data-action="sim-reset:${esc(simulator.id)}">Reset to closed</button></div>`;
    }).join('');
    return `<section class="panel" style="margin-top:28px"><p class="eyebrow">Development</p><h2>Simulator controls</h2><p class="subtle">Settings and resets apply only to the selected simulator.</p>${instances}</section>`;
}
function serviceStatusFields() {
    const scanner = state.diagnostics?.scanner;
    return [
        { label: 'Service', value: serviceConnectionLabel() },
        { label: 'Bluetooth adapter', value: serviceBooleanLabel(state.demoMode, 'Not used in demo', scanner?.adapter_available, 'Available', 'Unavailable') },
        { label: 'Scanner', value: serviceTextLabel(state.demoMode, 'Disabled in demo', scanner?.status, 'Unavailable') },
        { label: 'Advertisements seen', value: serviceDataLabel(state.demoMode, 'Not collected', scanner?.soma_advertisements_seen, '—') },
        { label: 'Last scanner error', value: serviceTextLabel(state.demoMode, 'Not applicable', scanner?.last_error_code, 'None reported') },
        { label: 'Telemetry history', value: state.diagnostics?.history_enabled ? 'Enabled' : 'Not enabled' },
        { label: 'Simulators', value: state.simulators?.length ?? 'Unavailable or disabled' },
    ];
}
function serviceBooleanLabel(demoMode, demoLabel, value, active, inactive) {
    if (demoMode)
        return demoLabel;
    return value ? active : inactive;
}
function serviceTextLabel(demoMode, demoLabel, value, fallback) {
    if (demoMode)
        return demoLabel;
    return value || fallback;
}
function serviceDataLabel(demoMode, demoLabel, value, fallback) {
    if (demoMode)
        return demoLabel;
    return value ?? fallback;
}
function serviceConnectionLabel() {
    if (state.serviceError)
        return 'Connection problem';
    if (!state.demoMode)
        return 'Connected';
    return state.diagnostics?.simulator_ready ? 'Demo ready · Simulated devices' : 'Simulator unavailable';
}
function servicePage() {
    const intro = servicePageIntro();
    const simulatorSection = simulatorControlsForMode();
    return `${back()}<h1 class="page-title">Service status</h1><p class="page-intro">${intro}</p>${renderInfoGrid(serviceStatusFields())}${simulatorSection}`;
}
function servicePageIntro() {
    if (state.demoMode)
        return 'A closer look at the Somaris service and simulated devices. The Bluetooth scanner is disabled for this demo.';
    return 'A closer look at the Somaris service and Bluetooth scanner.';
}
function simulatorControlsForMode() {
    return state.demoMode ? '' : simulatorControls();
}
function demoBanner() {
    return `<section class="demo-banner" aria-label="Somaris demonstration"><div><p class="eyebrow">Somaris Demo</p><p>Explore a simulated smart home. Your changes only affect this demonstration.</p></div><button type="button" class="button secondary" data-action="demo-reset">Reset demo</button></section>`;
}
function page() { return location.hash.slice(1) || 'home'; }
function pageContent(route) {
    if (route === 'add')
        return addPage();
    if (route === 'service')
        return servicePage();
    if (route.startsWith('details/'))
        return detailPage(decodeURIComponent(route.slice(8)));
    return home();
}
function topbarStatus() {
    const diagnostics = state.diagnostics;
    if (state.serviceError)
        return { serviceLabel: 'Disconnected', serviceTone: 'bad' };
    if (!diagnostics)
        return { serviceLabel: 'Connecting', serviceTone: '' };
    if (diagnostics.demo_mode && diagnostics.simulator_ready) {
        return { serviceLabel: 'Demo ready · Simulated devices', serviceTone: 'good' };
    }
    if (diagnostics.scanner?.adapter_available)
        return { serviceLabel: 'Connected', serviceTone: 'good' };
    return { serviceLabel: 'Service degraded', serviceTone: '' };
}
function updateTopbar() {
    const topActions = document.querySelector('#top-actions');
    if (!topActions)
        return;
    const activeElement = document.activeElement;
    const active = activeElement instanceof HTMLElement && topActions.contains(activeElement) ? activeElement : null;
    const focusId = active?.id || '';
    const focusLabel = active?.getAttribute('aria-label') || '';
    topActions.innerHTML = renderTopbarActions({
        blindCount: state.managed.length,
        panelMode: document.documentElement.dataset.displayMode === 'panel',
        displayPreference,
        ...topbarStatus(),
    });
    const replacement = focusId ? document.getElementById(focusId)
        : focusLabel ? [...topActions.querySelectorAll('[aria-label]')].find(el => el.getAttribute('aria-label') === focusLabel)
            : null;
    replacement?.focus({ preventScroll: true });
}
function restoreAppFocus(appRoot, focusedAction) {
    const actionTargets = [...appRoot.querySelectorAll('[data-action]')];
    const preferredAction = resolveActionFocus({
        focusedAction,
        pendingConfirmationId: state.pendingRemovalFocusId,
        confirmingRemovalId: state.confirmingRemovalId,
        availableActions: actionTargets.map(el => el.dataset.action ?? ''),
    });
    state.pendingRemovalFocusId = null;
    if (!preferredAction)
        return;
    actionTargets.find(element => element.dataset.action === preferredAction)?.focus({ preventScroll: true });
}
function render() {
    if (!app)
        return;
    const activeElement = document.activeElement;
    const focusedAction = activeElement instanceof HTMLElement && app.contains(activeElement)
        ? activeElement.dataset.action ?? null
        : null;
    const content = pageContent(page());
    app.innerHTML = `${state.demoMode ? demoBanner() : ''}${content}`;
    updateTopbar();
    restoreAppFocus(app, focusedAction);
}
export async function refreshService(target, request) {
    try {
        const [managed, observations, diagnostics, simulators, activity] = await Promise.all([
            request('/api/control/v1/devices'), request('/api/v1/devices'),
            request('/api/v1/diagnostics'), request('/api/dev/simulators').catch(() => null),
            request('/api/control/v1/audit-log'),
        ]);
        Object.assign(target, {
            managed: managed,
            observations: observations,
            diagnostics: diagnostics,
            demoMode: diagnostics.demo_mode === true,
            simulators: simulators,
            activity: activity,
        });
        serviceRecovered(target);
        const released = releaseReconciledCommandLocks(target);
        if (released)
            notifyTarget(target, { message: 'A fresh position is available. Verify it before trying again.', tone: 'warning' });
    }
    catch (error) {
        serviceFailure(target, error);
    }
}
export async function resetDemo(target, request, refreshData, confirmReset = () => true) {
    if (!confirmReset())
        return false;
    try {
        await request('/api/demo/reset', { method: 'POST' });
        if (target.commands)
            target.commands = {};
        if (target.batch !== undefined)
            target.batch = null;
        target.busy?.clear();
        target.reconciliationRequired?.clear();
        if (target.tokens)
            target.tokens = {};
        await refreshData();
        notifyTarget(target, { message: 'Your demo was reset to the original household.', tone: 'success' });
        return true;
    }
    catch (error) {
        notifyTarget(target, { message: `Could not reset the demo: ${errorMessage(error)}`, tone: 'error' });
        return false;
    }
}
function hasPostCommandFreshPosition(command, device) {
    const acceptedAt = Date.parse(command?.accepted_at || '');
    const observedAt = Date.parse(device?.position_observed_at || '');
    return device?.availability === 'ready'
        && device.position_freshness === 'fresh'
        && Number.isFinite(acceptedAt)
        && Number.isFinite(observedAt)
        && observedAt >= acceptedAt;
}
function releaseReconciledCommandLocks(target) {
    const required = target.reconciliationRequired;
    if (!(required instanceof Set))
        return false;
    let released = false;
    for (const id of required) {
        const command = target.commands?.[id];
        if (command && ['queued', 'connecting'].includes(command.status))
            continue;
        const device = target.managed?.find(item => item.enrollment?.id === id);
        if (!hasPostCommandFreshPosition(command, device))
            continue;
        required.delete(id);
        target.busy?.delete(id);
        released = true;
    }
    return released;
}
export async function refreshCommandTelemetry(target, request, managedDeviceId) {
    const [managedResult, observationsResult] = await Promise.allSettled([
        Promise.resolve().then(() => request('/api/control/v1/devices')),
        Promise.resolve().then(() => request('/api/v1/devices')),
    ]);
    const failures = [];
    if (managedResult.status === 'fulfilled')
        target.managed = managedResult.value;
    else
        failures.push(`Managed devices: ${errorMessage(managedResult.reason)}`);
    if (observationsResult.status === 'fulfilled')
        target.observations = observationsResult.value;
    else
        failures.push(`Observations: ${errorMessage(observationsResult.reason)}`);
    const device = managedResult.status === 'fulfilled'
        ? managedResult.value.find(item => item.enrollment?.id === managedDeviceId)
        : null;
    return {
        positionFresh: hasPostCommandFreshPosition(target.commands?.[managedDeviceId], device),
        errors: failures,
    };
}
export async function handleCommandPollFailure(target, id, token, error, refreshData, onChange = () => { }) {
    if (target.tokens?.[id] !== token)
        return false;
    const reason = errorMessage(error).replace(/[.!?]+$/, '');
    target.commands[id] = { ...target.commands[id], status: 'outcome_unknown' };
    target.reconciliationRequired ??= new Set();
    target.reconciliationRequired.add(id);
    onChange();
    const refreshResult = await refreshData();
    if (target.tokens?.[id] !== token)
        return false;
    const errors = refreshResult && refreshResult.errors?.length ? ` Some telemetry could not be refreshed: ${refreshResult.errors.join('; ')}.` : '';
    if (refreshResult && refreshResult.positionFresh === true) {
        target.reconciliationRequired.delete(id);
        target.busy?.delete(id);
        notifyTarget(target, { key: `command-status:${id}`, message: `Could not check command status: ${reason}. Check the latest reported position before trying again.${errors}`, tone: 'warning', action: { label: 'Refresh position', action: 'refresh' } });
    }
    else {
        notifyTarget(target, { key: `command-status:${id}`, message: `Could not check command status: ${reason}. A fresh managed-device position is unavailable; movement controls remain locked. Refresh and verify the position before trying again.${errors}`, tone: 'warning', action: { label: 'Refresh position', action: 'refresh' } });
    }
    onChange();
    return true;
}
export async function enrollBlind(target, request, refreshData, observedDeviceId, label) {
    try {
        await request('/api/control/v1/devices', {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ observed_device_id: observedDeviceId, label }),
        });
    }
    catch (error) {
        try {
            await refreshData();
        }
        catch (refreshError) {
            serviceFailure(target, refreshError);
        }
        const failure = apiError(error);
        const message = failure.code === 'simulator_limit_reached'
            ? 'Could not add simulator: You can add up to two simulators. Remove one from your managed blinds before adding another.'
            : `Could not add blind: ${failure.message.replace(/[.!?]+$/, "")}. Refresh and check the list before retrying.`;
        notifyTarget(target, { message, tone: 'error' });
        return false;
    }
    try {
        await refreshData();
    }
    catch (refreshError) {
        serviceFailure(target, refreshError);
    }
    notifyTarget(target, { message: `${label} added.`, tone: 'success' });
    return true;
}
export async function removeEnrolledBlind(target, request, refreshData, managedDeviceId) {
    if (target.removingEnrollmentId === managedDeviceId)
        return false;
    target.removingEnrollmentId = managedDeviceId;
    try {
        await request(`/api/control/v1/devices/${encodeURIComponent(managedDeviceId)}`, { method: 'DELETE' });
    }
    catch (error) {
        try {
            await refreshData();
        }
        catch (refreshError) {
            serviceFailure(target, refreshError);
        }
        const reason = errorMessage(error).replace(/[.!?]+$/, '');
        notifyTarget(target, { message: `Could not remove blind: ${reason}. Refresh the list and check the enrollment before retrying.`, tone: 'error' });
        target.removingEnrollmentId = null;
        return false;
    }
    const removed = target.managed?.find(device => device.enrollment.id === managedDeviceId);
    if (Array.isArray(target.managed)) {
        target.managed = target.managed.filter(device => device.enrollment.id !== managedDeviceId);
    }
    notifyTarget(target, { message: `${removed?.enrollment.label || 'Blind'} removed.`, tone: 'success' });
    if (target.commands)
        delete target.commands[managedDeviceId];
    target.busy?.delete(managedDeviceId);
    if (target.tokens)
        delete target.tokens[managedDeviceId];
    target.confirmingRemovalId = null;
    target.removingEnrollmentId = null;
    try {
        await refreshData();
    }
    catch (error) {
        serviceFailure(target, error);
    }
    return true;
}
async function refresh(quiet = false) { await refreshService(state, api); if (!quiet || !app?.querySelector('input:focus, select:focus'))
    render(); }
function notifyCommandResult(id, command) {
    const [message] = commandMessage(command);
    const label = state.managed.find(device => device.enrollment.id === id)?.enrollment.label;
    const tone = command.status === 'write_acknowledged' ? 'success' : ['connection_failed', 'expired'].includes(command.status) ? 'error' : 'warning';
    const notification = { key: `command:${command.id || id}:${command.status}`, message: label ? `${label}: ${message}` : message, tone };
    if (['outcome_unknown', 'write_failed'].includes(command.status))
        notification.action = { label: 'Refresh position', action: 'refresh' };
    snackbarQueue.push(notification);
}
export function buildCommandRequestBody(device, action, positionClosedPercent, motorSpeedPercent) {
    const expectedLabel = device.enrollment.label;
    if (typeof expectedLabel !== 'string' || !expectedLabel.trim() || expectedLabel.length > 64)
        throw new Error('Refresh the device list before sending a command.');
    const body = { action, expected_label: expectedLabel };
    if (action === 'set_position')
        body.position_closed_percent = positionClosedPercent;
    if (action === 'set_motor_speed')
        body.motor_speed_percent = motorSpeedPercent;
    return body;
}
async function sendCommand(id, action, positionClosedPercent, motorSpeedPercent) {
    if ((state.busy.has(id) || state.reconciliationRequired.has(id) || blindBatchIsLocked(state.batch)) && action !== 'stop')
        return;
    const target = state.managed.find(device => device.enrollment.id === id);
    if (!target?.enrollment.label) {
        snackbarQueue.push({ key: `command-target:${id}`, message: 'Refresh the blind list before sending a command.', tone: 'warning' });
        render();
        return;
    }
    const token = Symbol();
    state.busy.add(id);
    state.tokens[id] = token;
    state.commands[id] = { status: 'queued' };
    render();
    try {
        const key = globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random().toString(36).slice(2)}`;
        const body = buildCommandRequestBody(target, action, positionClosedPercent, motorSpeedPercent);
        const command = await api(`/api/control/v1/devices/${encodeURIComponent(id)}/commands`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Idempotency-Key': key }, body: JSON.stringify(body) });
        if (!command.id)
            throw new Error('The service returned a command without an ID.');
        if (state.tokens[id] !== token)
            return;
        state.commands[id] = command;
        render();
        pollCommand(id, command.id, token);
    }
    catch (error) {
        if (state.tokens[id] !== token)
            return;
        state.commands[id] = { status: 'outcome_unknown' };
        snackbarQueue.push({ key: `command-request:${id}`, message: `Command request failed: ${errorMessage(error)}. Its outcome may be unknown; check telemetry before trying again.`, tone: 'warning', action: { label: 'Refresh position', action: 'refresh' } });
        if (!state.reconciliationRequired.has(id))
            state.busy.delete(id);
        render();
    }
}
async function pollCommand(id, commandId, token) {
    try {
        const command = await api(`/api/control/v1/commands/${encodeURIComponent(commandId)}`);
        if (state.tokens[id] !== token)
            return;
        state.commands[id] = command;
        render();
        if (!terminal.has(command.status)) {
            setTimeout(() => pollCommand(id, commandId, token), 1500);
            return;
        }
        if (['outcome_unknown', 'write_failed'].includes(command.status))
            state.reconciliationRequired.add(id);
        if (!state.reconciliationRequired.has(id))
            state.busy.delete(id);
        notifyCommandResult(id, command);
        await refresh(true);
    }
    catch (error) {
        await handleCommandPollFailure(state, id, token, error, () => refreshCommandTelemetry(state, api, id), render);
    }
}
function pollBlindBatch(batch) {
    setTimeout(async () => {
        if (state.batch !== batch || !batch.id || !blindBatchIsLocked(batch))
            return;
        const updated = await checkBlindBatch(state, api, render);
        if (!updated || state.batch !== batch)
            return;
        if (!blindBatchIsLocked(batch)) {
            await refresh();
            return;
        }
        pollBlindBatch(batch);
    }, 1500);
}
async function startBlindBatch(action) {
    const accepted = await submitBlindBatch(state, api, action, render);
    if (!accepted) {
        if (state.batch?.status === 'rejected')
            await refresh();
        return;
    }
    const batch = state.batch;
    if (batch?.id && blindBatchIsLocked(batch))
        pollBlindBatch(batch);
    else
        await refresh();
}
async function retryBlindBatchCheck() {
    const batch = state.batch;
    if (!batch)
        return;
    const updated = await checkBlindBatch(state, api, render);
    if (!updated || state.batch !== batch)
        return;
    if (blindBatchIsLocked(batch) && batch.id)
        pollBlindBatch(batch);
    else if (!blindBatchIsLocked(batch))
        await refresh();
}
async function simulatorReset(id) { try {
    await api(`/api/dev/simulators/${encodeURIComponent(id)}/reset`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ position_closed_percent: 100 }) });
    await refresh();
    snackbarQueue.push({ message: 'Simulator reset.', tone: 'success' });
}
catch (error) {
    snackbarQueue.push({ message: `Simulator reset failed: ${errorMessage(error)}`, tone: 'error' });
    render();
} }
async function confirmEnrollmentRemoval(id) {
    if (state.removingEnrollmentId === id)
        return;
    const removal = removeEnrolledBlind(state, api, () => refreshService(state, api), id);
    render();
    if (await removal) {
        location.hash = 'home';
        return;
    }
    render();
}
function closestHtmlElement(target, selector) {
    return target instanceof Element ? target.closest(selector) : null;
}
function containsRelatedTarget(element, target) {
    return target instanceof Node && element.contains(target);
}
export function parseActionValue(value) {
    const [action = '', argument = '', id = ''] = value?.split(':') ?? [];
    return [action, argument, id];
}
function isBlindAction(action) {
    return action !== undefined && [
        'open', 'close', 'stop', 'toggle', 'set_position', 'set_motor_speed',
        'calibration_start', 'calibration_nudge_open', 'calibration_set_open_limit',
        'calibration_nudge_closed', 'calibration_set_closed_limit', 'calibration_stop',
    ].includes(action);
}
export function dispatchBlindCommand(action, encodedId, send = sendCommand) {
    if (!encodedId || !isBlindAction(action))
        return false;
    void send(decodeURIComponent(encodedId), action);
    return true;
}
function changeDisplayMode(applyDisplayMode) {
    displayPreference = nextDisplayPreference(displayPreference);
    try {
        globalThis.localStorage?.setItem('somaris-display-mode', displayPreference);
    }
    catch { /* The current selection still applies until the page reloads. */ }
    applyDisplayMode();
    render();
    const label = DISPLAY_MODE_OPTIONS.find(([value]) => value === displayPreference)?.[1] || 'Auto';
    snackbarQueue.push({ message: `View changed to ${label}.`, tone: 'info' });
}
function handleBatchClick(argument) {
    if (argument === 'check')
        void retryBlindBatchCheck();
    else if (argument === 'open' || argument === 'close')
        void startBlindBatch(argument);
    return true;
}
function handlePositionClick(argument) {
    if (document.documentElement.dataset.displayMode === 'panel')
        return false;
    const input = app?.querySelector(`[data-position-input="${CSS.escape(argument)}"]`);
    const position = Number(input?.value);
    if (Number.isInteger(position) && position >= 0 && position <= 100)
        void sendCommand(argument, 'set_position', position);
    return true;
}
function handleRemovalClick(action, deviceId) {
    if (action === 'remove') {
        state.confirmingRemovalId = deviceId;
        state.pendingRemovalFocusId = deviceId;
        render();
        return true;
    }
    if (action === 'remove-confirm') {
        void confirmEnrollmentRemoval(deviceId);
        return true;
    }
    if (action === 'remove-cancel' && state.removingEnrollmentId !== deviceId) {
        state.confirmingRemovalId = null;
        state.pendingRemovalFocusId = null;
        render();
    }
    return true;
}
const clickActionHandlers = {
    'snackbar-dismiss': (_argument, _id, deviceId) => { snackbarQueue.dismiss(deviceId); return true; },
    'display-mode': (_argument, _id, _deviceId, applyDisplayMode) => { changeDisplayMode(applyDisplayMode); return true; },
    command: (argument, id) => dispatchBlindCommand(argument, id),
    batch: argument => handleBatchClick(argument),
    position: argument => handlePositionClick(argument),
    details: argument => {
        if (!argument)
            return false;
        location.hash = `details/${encodeURIComponent(argument)}`;
        return true;
    },
    refresh: () => { void refresh(); return true; },
    'sim-reset': argument => {
        if (!argument)
            return false;
        void simulatorReset(argument);
        return true;
    },
    'demo-reset': () => {
        void resetDemo(state, api, () => refreshService(state, api), () => globalThis.confirm('Reset your demo? This restores all four blinds and clears this session’s activity.')).then(() => render());
        return true;
    },
    remove: (_argument, _id, deviceId) => handleRemovalClick('remove', deviceId),
    'remove-confirm': (_argument, _id, deviceId) => handleRemovalClick('remove-confirm', deviceId),
    'remove-cancel': (_argument, _id, deviceId) => handleRemovalClick('remove-cancel', deviceId),
};
function dispatchClickAction(action, argument, id, applyDisplayMode) {
    const deviceId = argument ? decodeURIComponent(argument) : '';
    const handler = clickActionHandlers[action];
    if (!handler || !handler(argument, id, deviceId, applyDisplayMode))
        location.hash = action;
}
async function saveSimulatorSettings(form) {
    const id = form.dataset.simulator;
    if (!id)
        return;
    const values = Object.fromEntries(new FormData(form).entries());
    try {
        await api(`/api/dev/simulators/${encodeURIComponent(id)}/scenario`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(values) });
        await refresh();
        snackbarQueue.push({ message: 'Simulator settings saved.', tone: 'success' });
    }
    catch (error) {
        snackbarQueue.push({ message: `Simulator settings could not be saved: ${errorMessage(error)}`, tone: 'error' });
        render();
    }
}
function submitMotorSpeed(form) {
    const id = form.dataset.motorSpeed;
    if (!id)
        return;
    const speed = Number(new FormData(form).get('motor_speed_percent'));
    if (Number.isInteger(speed) && speed >= 0 && speed <= 100)
        void sendCommand(id, 'set_motor_speed', undefined, speed);
}
async function submitEnrollmentForm(form) {
    const button = form.querySelector('button');
    if (button)
        button.disabled = true;
    const id = form.dataset.enroll;
    const labelValue = new FormData(form).get('label');
    const label = typeof labelValue === 'string' ? labelValue.trim() : '';
    if (!id || !label)
        return;
    const added = await enrollBlind(state, api, () => refreshService(state, api), id, label);
    if (added)
        location.hash = 'home';
    render();
}
async function handleFormSubmit(event) {
    const form = event.target instanceof HTMLFormElement ? event.target : null;
    if (!form)
        return;
    if (form.matches('[data-simulator]')) {
        event.preventDefault();
        await saveSimulatorSettings(form);
        return;
    }
    if (form.matches('[data-motor-speed]')) {
        event.preventDefault();
        submitMotorSpeed(form);
        return;
    }
    if (!form.matches('[data-enroll]'))
        return;
    event.preventDefault();
    await submitEnrollmentForm(form);
}
if (app) {
    const actionRoot = document.querySelector('.app-shell');
    if (actionRoot) {
        const applyDisplayMode = () => {
            const previous = document.documentElement.dataset.displayMode;
            document.documentElement.dataset.displayMode = resolveDisplayMode(displayPreference, window.innerWidth, window.innerHeight);
            return previous !== document.documentElement.dataset.displayMode;
        };
        applyDisplayMode();
        let initialized = false;
        window.addEventListener('resize', () => { if (applyDisplayMode() && initialized)
            render(); });
        window.addEventListener('hashchange', () => { if (initialized)
            render(); });
        actionRoot.addEventListener('click', (event) => {
            const target = closestHtmlElement(event.target, '[data-action]');
            if (!target)
                return;
            const [action, argument, id] = parseActionValue(target.dataset.action);
            dispatchClickAction(action, argument, id, applyDisplayMode);
        });
        snackbarRoot?.addEventListener('pointerover', (event) => {
            const item = closestHtmlElement(event.target, '[data-snackbar-id]');
            if (item && !containsRelatedTarget(item, event.relatedTarget))
                snackbarQueue.pause(item.dataset.snackbarId ?? '', 'pointer');
        });
        snackbarRoot?.addEventListener('pointerout', (event) => {
            const item = closestHtmlElement(event.target, '[data-snackbar-id]');
            if (item && !containsRelatedTarget(item, event.relatedTarget))
                snackbarQueue.resume(item.dataset.snackbarId ?? '', 'pointer');
        });
        snackbarRoot?.addEventListener('focusin', (event) => {
            const item = closestHtmlElement(event.target, '[data-snackbar-id]');
            if (item)
                snackbarQueue.pause(item.dataset.snackbarId ?? '', 'focus');
        });
        snackbarRoot?.addEventListener('focusout', (event) => {
            const item = closestHtmlElement(event.target, '[data-snackbar-id]');
            if (item && !containsRelatedTarget(item, event.relatedTarget))
                snackbarQueue.resume(item.dataset.snackbarId ?? '', 'focus');
        });
        actionRoot.addEventListener('input', (event) => {
            const input = closestHtmlElement(event.target, '[data-position-input]');
            if (!input)
                return;
            const range = input instanceof HTMLInputElement ? input : null;
            if (!range)
                return;
            const output = app.querySelector(`[data-position-output="${CSS.escape(range.dataset.positionInput ?? '')}"]`);
            if (output)
                output.value = `${range.value}%`;
        });
        app.addEventListener('submit', (event) => void handleFormSubmit(event));
        const initializeDashboard = async () => {
            try {
                const mode = await api('/api/v1/demo');
                state.demoMode = mode.demo_mode === true;
                if (state.demoMode)
                    await api('/api/demo/session', { method: 'POST' });
                await refresh();
            }
            catch (error) {
                serviceFailure(state, error);
                render();
            }
            finally {
                initialized = true;
            }
        };
        void initializeDashboard();
        setInterval(() => { if (initialized)
            void refresh(true); }, 5000);
    }
}
