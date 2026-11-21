import {escapeHtml} from './escape.mjs';

interface DeviceListOptions {
  count: number;
  panelMode?: boolean;
  cardsMarkup?: string;
  emptyMarkup?: string;
  toolbarMarkup?: string;
  paginationMarkup?: string;
  groupsMarkup?: string;
  title?: string;
}

interface InfoItemOptions {
  label: string;
  value?: string | number | null;
  contentMarkup?: string;
}

export function renderDeviceList({count, panelMode = false, cardsMarkup = '', emptyMarkup = '', toolbarMarkup = '', paginationMarkup = '', groupsMarkup = '', title = 'Your blinds'}: DeviceListOptions): string {
  const titleContent = panelMode ? '' : `<h2>${escapeHtml(title)}</h2>`;
  const countLabel = panelMode ? '' : `<span class="count">${count} ${count === 1 ? 'blind' : 'blinds'}</span>`;
  const content = groupsMarkup || (cardsMarkup ? `<div class="grid">${cardsMarkup}</div>` : emptyMarkup);
  const heading = panelMode ? '' : `<div class="section-heading device-list-heading">${titleContent}${countLabel}</div>`;
  return `<section class="device-list${panelMode ? ' device-list--panel' : ''}" aria-label="${escapeHtml(title)}">${heading}${toolbarMarkup}${content}${paginationMarkup}</section>`;
}

export function renderHomeSupportingSections({panelMode = false, activityMarkup = '', serviceMarkup = ''}: {panelMode?: boolean; activityMarkup?: string; serviceMarkup?: string}): string {
  return panelMode ? '' : `${activityMarkup}${serviceMarkup}`;
}

function renderInfoItem({label, value, contentMarkup}: InfoItemOptions): string {
  const content = contentMarkup === undefined ? `<strong>${escapeHtml(value)}</strong>` : contentMarkup;
  return `<div class="info-item"><span class="meta-label subtle">${escapeHtml(label)}</span>${content}</div>`;
}

export function renderInfoGrid(items: InfoItemOptions[]): string {
  return `<div class="info-grid">${items.map(renderInfoItem).join('')}</div>`;
}
