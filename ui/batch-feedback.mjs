const batchFailureStatuses = new Set(['connection_failed', 'deferred', 'write_failed', 'outcome_unknown', 'expired', 'cancelled']);
const batchSkipLabels = {
    not_seen: 'not seen recently',
    stale: 'stale observation',
    ambiguous: 'ambiguous device',
    cooldown: 'movement cooldown',
    calibration_active: 'calibration active',
};
function runningBatchFeedback(action, data) {
    const acknowledged = data.acknowledged_count || 0;
    const total = data.target_count || 0;
    const failedCount = data.failed_count || 0;
    const pending = data.pending_count || 0;
    const movement = action === 'close' ? 'Closing' : 'Opening';
    const failed = failedCount ? `${failedCount} failed, ` : '';
    return {
        text: `${movement} blinds: commands acknowledged for ${acknowledged} of ${total} ready blinds; ${failed}${pending} pending.`,
        tone: '',
    };
}
function batchFeedbackMessage(action, status, data) {
    const actionLabel = action === 'close' ? 'Close' : 'Open';
    switch (status) {
        case 'submitting':
            return { text: `Sending ${actionLabel.toLowerCase()} commands to ready blinds…`, tone: '' };
        case 'checking':
            return { text: `Checking the ${actionLabel.toLowerCase()} batch…`, tone: '' };
        case 'outcome_unknown':
            return { text: `The ${actionLabel.toLowerCase()} batch result is unknown. Check its status before starting another batch.`, tone: 'warn' };
        case 'queued':
            return { text: `${actionLabel} batch queued for ${data.target_count || 0} ready blinds.`, tone: '' };
        case 'running':
            return runningBatchFeedback(action, data);
        default:
            return null;
    }
}
function skippedBatchMarkup(items, escapeHtml) {
    if (!items?.length)
        return '';
    const rows = items.map(item => {
        const status = batchSkipLabels[item.availability ?? ''] || item.availability || 'unavailable';
        return `<li>${escapeHtml(item.label)} · ${escapeHtml(status)}</li>`;
    }).join('');
    return `<div class="batch-skipped"><span>Skipped blinds</span><ul>${rows}</ul></div>`;
}
function failedBatchMarkup(targets, escapeHtml) {
    const failures = targets?.filter(target => batchFailureStatuses.has(target.status)) ?? [];
    if (!failures.length)
        return '';
    const rows = failures.map(item => `<li>${escapeHtml(item.label)} · ${escapeHtml(item.status.replaceAll('_', ' '))}</li>`).join('');
    return `<div class="batch-failures"><span>Unsuccessful blinds</span><ul>${rows}</ul></div>`;
}
export function renderBatchFeedbackMarkup(batch, escapeHtml, renderButton) {
    if (!batch)
        return '';
    const data = batch.data || batch;
    const action = batch.action || data.action;
    const status = batch.status || data.status;
    const content = batchFeedbackMessage(action, status, data);
    if (!content)
        return '';
    const retry = status === 'outcome_unknown'
        ? renderButton({ label: batch.id ? 'Check batch status' : 'Retry original request', action: 'batch:check', variant: 'secondary' })
        : '';
    const errorDetail = status === 'outcome_unknown' && batch.error
        ? `<span class="subtle">${escapeHtml(batch.error)}</span>`
        : '';
    return `<div class="batch-feedback${content.tone ? ` batch-feedback--${content.tone}` : ''}" role="status" aria-live="polite"><p>${escapeHtml(content.text)}</p>${errorDetail}${skippedBatchMarkup(data.skipped, escapeHtml)}${failedBatchMarkup(data.targets, escapeHtml)}${retry}</div>`;
}
