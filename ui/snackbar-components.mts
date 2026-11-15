import type {SnackbarAction, Tone} from './types.mjs';
import {escapeHtml} from './escape.mjs';

interface SnackbarRenderOptions {
  id?: string;
  message?: string;
  tone?: Tone;
  action?: SnackbarAction | null;
  details?: string[];
}

export function renderSnackbar({id, message, tone = 'info', action = null, details = []}: SnackbarRenderOptions = {}): string {
  if (typeof message !== 'string' || !message.trim()) return '';
  const safeTone: string = ['info', 'success', 'warning', 'error'].includes(tone ?? '') ? tone ?? 'info' : 'info';
  const isError = safeTone === 'error';
  const actionMarkup = action && typeof action.label === 'string' && typeof action.action === 'string'
    ? `<button type="button" class="snackbar-action" data-snackbar-control="action" data-action="${escapeHtml(action.action)}">${escapeHtml(action.label)}</button>`
    : '';
  const detailItems = Array.isArray(details) && details.length
    ? `<ul class="snackbar-details">${details.map(detail => `<li>${escapeHtml(detail)}</li>`).join('')}</ul>`
    : '';
  return `<article class="snackbar snackbar--${safeTone}" data-snackbar-id="${escapeHtml(id || '')}" role="${isError ? 'alert' : 'status'}" aria-live="${isError ? 'assertive' : 'polite'}" aria-atomic="true"><span class="snackbar-message">${escapeHtml(message)}</span>${detailItems}${actionMarkup}<button type="button" class="snackbar-dismiss" data-snackbar-control="dismiss" data-action="snackbar-dismiss:${escapeHtml(id || '')}" aria-label="Dismiss notification" title="Dismiss">×</button></article>`;
}

export function renderSnackbarRegion(items: SnackbarRenderOptions[] = []): string {
  return `<div id="snackbar-region" class="snackbar-region" role="region" aria-label="Notifications">${(items || []).map(renderSnackbar).join('')}</div>`;
}
