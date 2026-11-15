import { escapeHtml } from './escape.mjs';
export function renderDeviceList({ count, panelMode = false, cardsMarkup = '', emptyMarkup = '', toolbarMarkup = '', title = 'Your blinds' }) {
    const titleContent = panelMode ? '' : `<h2>${escapeHtml(title)}</h2>`;
    const countLabel = panelMode ? '' : `<span class="count">${count} ${count === 1 ? 'blind' : 'blinds'}</span>`;
    const content = cardsMarkup ? `<div class="grid">${cardsMarkup}</div>` : emptyMarkup;
    const heading = panelMode ? '' : `<div class="section-heading device-list-heading">${titleContent}${countLabel}</div>`;
    return `<section class="device-list${panelMode ? ' device-list--panel' : ''}" aria-label="${escapeHtml(title)}">${heading}${toolbarMarkup}${content}</section>`;
}
export function renderHomeSupportingSections({ panelMode = false, activityMarkup = '', serviceMarkup = '' }) {
    return panelMode ? '' : `${activityMarkup}${serviceMarkup}`;
}
function renderInfoItem({ label, value, contentMarkup }) {
    const content = contentMarkup === undefined ? `<strong>${escapeHtml(value)}</strong>` : contentMarkup;
    return `<div class="info-item"><span class="meta-label subtle">${escapeHtml(label)}</span>${content}</div>`;
}
export function renderInfoGrid(items) {
    return `<div class="info-grid">${items.map(renderInfoItem).join('')}</div>`;
}
