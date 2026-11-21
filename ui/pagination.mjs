import { escapeHtml } from './escape.mjs';
export const UI_PAGE_SIZE = 24;
export function windowPage(items, visibleCount) {
    const limit = Number.isFinite(visibleCount) ? Math.max(0, Math.floor(visibleCount)) : 0;
    const total = items.length;
    return {
        items: items.slice(0, limit),
        total,
        hasMore: limit < total,
        nextLimit: Math.min(total, limit + UI_PAGE_SIZE),
    };
}
export function renderLoadMore({ shown, total, action, singular, plural }) {
    if (shown >= total)
        return '';
    const nextCount = Math.min(UI_PAGE_SIZE, total - shown);
    const nextLabel = nextCount === 1 ? singular : plural;
    return `<div class="list-pagination"><span class="subtle">Showing ${shown} of ${total} ${escapeHtml(plural)}</span><button class="button secondary" data-action="${escapeHtml(action)}">Show ${nextCount} more ${escapeHtml(nextLabel)}</button></div>`;
}
