import { escapeHtml } from './escape.mjs';
export function renderSnackbar({ id, message, tone = 'info', action = null, details = [] } = {}) {
    if (typeof message !== 'string' || !message.trim())
        return '';
    const safeTone = ['info', 'success', 'warning', 'error'].includes(tone ?? '') ? tone ?? 'info' : 'info';
    const isError = safeTone === 'error';
    const actionIcons = {
        undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h9a7 7 0 0 1 0 14h-2" transform="translate(0 -2)"/>',
        refresh: '<path d="M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4"/>',
        check: '<circle cx="12" cy="12" r="9"/><path d="m8 12 2.5 2.5L16.5 9"/>',
        warning: '<path d="M12 3 22 20H2L12 3Z"/><path d="M12 9v5m0 3h.01"/>',
    };
    const iconMarkup = action?.icon && actionIcons[action.icon]
        ? `<svg class="ui-icon" data-icon="${escapeHtml(action.icon)}" viewBox="0 0 24 24" aria-hidden="true" focusable="false">${actionIcons[action.icon]}</svg>`
        : '';
    const actionMarkup = action && typeof action.label === 'string' && typeof action.action === 'string'
        ? `<button type="button" class="snackbar-action${iconMarkup ? ' snackbar-action--icon' : ''}" data-snackbar-control="action" data-action="${escapeHtml(action.action)}"${iconMarkup ? ` aria-label="${escapeHtml(action.label)}" title="${escapeHtml(action.label)}"` : ''}>${iconMarkup}${iconMarkup ? '' : escapeHtml(action.label)}</button>`
        : '';
    const detailItems = Array.isArray(details) && details.length
        ? `<ul class="snackbar-details">${details.map(detail => `<li>${escapeHtml(detail)}</li>`).join('')}</ul>`
        : '';
    return `<article class="snackbar snackbar--${safeTone}" data-snackbar-id="${escapeHtml(id || '')}" role="${isError ? 'alert' : 'status'}" aria-live="${isError ? 'assertive' : 'polite'}" aria-atomic="true"><span class="snackbar-message">${escapeHtml(message)}</span>${detailItems}${actionMarkup}<button type="button" class="snackbar-dismiss" data-snackbar-control="dismiss" data-action="snackbar-dismiss:${escapeHtml(id || '')}" aria-label="Dismiss notification" title="Dismiss">×</button></article>`;
}
export function renderSnackbarRegion(items = []) {
    return `<div id="snackbar-region" class="snackbar-region" role="region" aria-label="Notifications">${(items || []).map(renderSnackbar).join('')}</div>`;
}
