import { renderAddDeviceEmptyState, renderBackButton, renderButton, renderRoomPicker } from './components.mjs';
import { escapeHtml } from './escape.mjs';
import { windowPage, renderLoadMore } from './pagination.mjs';
export function renderAddDevicePage({ observations, managed, rooms, demoMode, adapterAvailable, visibleCount, now = Date.now(), }) {
    const intro = demoMode
        ? 'This demo lists simulated blinds only; Bluetooth discovery is unavailable here.'
        : 'Choose a recently discovered blind to make it available for control.';
    const enrolled = new Set(managed.map(device => `${device.enrollment.logical_id}\u0000${!!device.enrollment.is_simulated}`));
    const available = observations.filter(observation => !enrolled.has(`${observation.logical_id}\u0000${!!observation.is_simulated}`));
    const page = windowPage(available, visibleCount);
    const nearby = page.total
        ? `<div class="section-heading"><h2>Nearby devices</h2><span class="count">${page.total} found</span></div><div class="stack">${page.items.map(observation => {
            const deviceName = observation.advertised_name || observation.device_name || 'Unnamed blind';
            const safeName = escapeHtml(deviceName);
            const safeId = escapeHtml(observation.id);
            const fieldId = `display-name-${encodeURIComponent(observation.id)}`;
            const safeFieldId = escapeHtml(fieldId);
            const errorId = `${fieldId}-error`;
            const safeErrorId = escapeHtml(errorId);
            const safeValue = escapeHtml(observation.advertised_name || observation.device_name || 'Blind');
            return `<form class="panel device-row" data-enroll="${safeId}" novalidate><div><h3>${safeName}</h3><p class="subtle">${observation.is_simulated ? 'Simulator · ' : ''}${relativeAt(observation.last_seen_at, now)} · ${escapeHtml(observation.freshness)}</p><label class="field" for="${safeFieldId}">Display name<input id="${safeFieldId}" name="label" maxlength="64" required value="${safeValue}" aria-describedby="${safeErrorId}" aria-invalid="false" data-display-name><span class="field-error" id="${safeErrorId}" hidden role="alert"></span></label>${renderRoomPicker({ id: `room-choice-${encodeURIComponent(observation.id)}`, rooms })}</div>${renderButton({ label: 'Add blind', accessibleLabel: `Add ${deviceName}`, title: `Add ${deviceName}`, type: 'submit', icon: 'add', iconOnly: true, variant: 'secondary', disabled: observation.freshness !== 'fresh' })}</form>`;
        }).join('')}</div>${renderLoadMore({ shown: page.items.length, total: page.total, action: 'more-nearby', singular: 'device', plural: 'devices' })}`
        : demoMode
            ? renderAddDeviceEmptyState({ demoMode: true, adapterAvailable: false })
            : `<div class="section-heading"><h2>Nearby devices</h2><span class="count">0 found</span></div>${renderAddDeviceEmptyState({ demoMode: false, adapterAvailable })}`;
    return `${renderBackButton()}<h1 class="page-title">Add a blind</h1><p class="page-intro">${intro}</p>${nearby}`;
}
function relativeAt(date, now) {
    if (!date)
        return 'Never heard from';
    const seconds = Math.max(0, Math.floor((now - new Date(date).getTime()) / 1000));
    if (!Number.isFinite(seconds))
        return 'Time unavailable';
    if (seconds < 60)
        return 'Last heard from just now';
    if (seconds < 3600)
        return `Last heard from ${Math.floor(seconds / 60)} min ago`;
    if (seconds < 86400)
        return `Last heard from ${Math.floor(seconds / 3600)} hr ago`;
    return `Last heard from ${Math.floor(seconds / 86400)} days ago`;
}
