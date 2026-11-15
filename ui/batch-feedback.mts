import type {BatchAction, BatchResponse} from './types.mjs';

export interface BatchFeedbackView {
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

interface BatchFeedbackMessage {
  text: string;
  tone: 'warn' | '';
}

type EscapeHtml = (value: unknown) => string;
type RetryButtonRenderer = (options: {label: string; action: string; variant?: string}) => string;

const batchFailureStatuses = new Set(['connection_failed', 'deferred', 'write_failed', 'outcome_unknown', 'expired', 'cancelled']);
const batchSkipLabels: Record<string, string> = {
  not_seen:'not seen recently',
  stale:'stale observation',
  ambiguous:'ambiguous device',
  cooldown:'movement cooldown',
  calibration_active:'calibration active',
};

function runningBatchFeedback(action: BatchAction | undefined, data: BatchFeedbackView): BatchFeedbackMessage {
  const acknowledged = data.acknowledged_count || 0;
  const total = data.target_count || 0;
  const failedCount = data.failed_count || 0;
  const pending = data.pending_count || 0;
  const movement = action === 'close' ? 'Closing' : 'Opening';
  const failed = failedCount ? `${failedCount} failed, ` : '';
  return {
    text:`${movement} blinds: commands acknowledged for ${acknowledged} of ${total} ready blinds; ${failed}${pending} pending.`,
    tone:'',
  };
}

function batchFeedbackMessage(
  action: BatchAction | undefined,
  status: string | undefined,
  data: BatchFeedbackView,
): BatchFeedbackMessage | null {
  const actionLabel = action === 'close' ? 'Close' : 'Open';
  switch (status) {
    case 'submitting':
      return {text:`Sending ${actionLabel.toLowerCase()} commands to ready blinds…`, tone:''};
    case 'checking':
      return {text:`Checking the ${actionLabel.toLowerCase()} batch…`, tone:''};
    case 'outcome_unknown':
      return {text:`The ${actionLabel.toLowerCase()} batch result is unknown. Check its status before starting another batch.`, tone:'warn'};
    case 'queued':
      return {text:`${actionLabel} batch queued for ${data.target_count || 0} ready blinds.`, tone:''};
    case 'running':
      return runningBatchFeedback(action,data);
    default:
      return null;
  }
}

function skippedBatchMarkup(items: BatchFeedbackView['skipped'], escapeHtml: EscapeHtml): string {
  if (!items?.length) return '';
  const rows = items.map(item => {
    const status = batchSkipLabels[item.availability ?? ''] || item.availability || 'unavailable';
    return `<li>${escapeHtml(item.label)} · ${escapeHtml(status)}</li>`;
  }).join('');
  return `<div class="batch-skipped"><span>Skipped blinds</span><ul>${rows}</ul></div>`;
}

function failedBatchMarkup(targets: BatchFeedbackView['targets'], escapeHtml: EscapeHtml): string {
  const failures = targets?.filter(target => batchFailureStatuses.has(target.status)) ?? [];
  if (!failures.length) return '';
  const rows = failures.map(item => `<li>${escapeHtml(item.label)} · ${escapeHtml(item.status.replaceAll('_', ' '))}</li>`).join('');
  return `<div class="batch-failures"><span>Unsuccessful blinds</span><ul>${rows}</ul></div>`;
}

export function renderBatchFeedbackMarkup(
  batch: BatchFeedbackView | null | undefined,
  escapeHtml: EscapeHtml,
  renderButton: RetryButtonRenderer,
): string {
  if (!batch) return '';
  const data: BatchFeedbackView = batch.data || batch;
  const action = batch.action || data.action;
  const status = batch.status || data.status;
  const content = batchFeedbackMessage(action, status, data);
  if (!content) return '';
  const retry = status === 'outcome_unknown'
    ? renderButton({label:batch.id ? 'Check batch status' : 'Retry original request', action:'batch:check', variant:'secondary'})
    : '';
  const errorDetail = status === 'outcome_unknown' && batch.error
    ? `<span class="subtle">${escapeHtml(batch.error)}</span>`
    : '';
  return `<div class="batch-feedback${content.tone ? ` batch-feedback--${content.tone}` : ''}" role="status" aria-live="polite"><p>${escapeHtml(content.text)}</p>${errorDetail}${skippedBatchMarkup(data.skipped,escapeHtml)}${failedBatchMarkup(data.targets,escapeHtml)}${retry}</div>`;
}
