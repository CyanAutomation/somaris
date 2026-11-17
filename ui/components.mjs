import { escapeHtml } from './escape.mjs';
import { renderBatchFeedbackMarkup } from './batch-feedback.mjs';
export { escapeHtml };
export { renderDeviceList, renderHomeSupportingSections, renderInfoGrid } from './page-components.mjs';
export { renderSnackbar, renderSnackbarRegion } from './snackbar-components.mjs';
const icons = {
    add: '<path d="M12 5v14M5 12h14"/>',
    activity: '<path d="M3 12h4l3-8 4 16 3-8h4"/>',
    service: '<path d="M4 5h16v14H4z"/><path d="M8 9h8m-8 4h5"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.42 1.42m11.3 11.3 1.42 1.42M2 12h2m16 0h2M4.93 19.07l1.42-1.42m11.3-11.3 1.42-1.42"/>',
    moon: '<path d="M20.5 15.5A8.5 8.5 0 0 1 8.5 3.5 8.5 8.5 0 1 0 20.5 15.5Z"/>',
    contrast: '<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 0 0 18z"/>',
    mobile: '<rect x="6" y="2.5" width="12" height="19" rx="2"/><path d="M10 18.5h4"/>',
    desktop: '<rect x="3" y="4" width="18" height="13" rx="1.5"/><path d="M8 21h8m-4-4v4"/>',
    panel: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 3v18M3 12h18"/>',
    chevron: '<path d="m7 10 5 5 5-5"/>',
    undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h9a7 7 0 0 1 0 14h-2" transform="translate(0 -2)"/>',
    reset: '<path d="M4 7v5h5M5.5 12a7 7 0 1 0 2.1-5"/><path d="M12 8v4l3 2"/>',
    spinner: '<path d="M20 12a8 8 0 1 1-2.34-5.66"/>',
    check: '<circle cx="12" cy="12" r="9"/><path d="m8 12 2.5 2.5L16.5 9"/>',
    warning: '<path d="M12 3 22 20H2L12 3Z"/><path d="M12 9v5m0 3h.01"/>',
    home: '<path d="m3 11 9-8 9 8v10h-6v-6H9v6H3z"/>',
    position0: '<rect x="4" y="3" width="16" height="18" rx="1.5"/>',
    position1: '<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 8h14"/>',
    position2: '<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 8h14m-14 5h14"/>',
    position3: '<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 7h14m-14 5h14m-14 5h14"/>',
    position4: '<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 6h14m-14 4h14m-14 4h14m-14 4h14"/>',
    back: '<path d="M19 12H5m7 7-7-7 7-7"/>',
    display: '<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5m0-8h.01"/>',
    observed: '<circle cx="12" cy="12" r="8.5"/><path d="m8.5 12 2.3 2.3 4.8-5"/>',
    stale: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3 2"/>',
    not_seen: '<circle cx="12" cy="12" r="8.5"/><path d="m9 9 6 6m0-6-6 6"/>',
    ambiguous: '<path d="M12 3.5 22 20H2L12 3.5Z"/><path d="M12 9v5m0 3h.01"/>',
    unavailable: '<circle cx="12" cy="12" r="8.5"/><path d="M9.8 9a2.3 2.3 0 1 1 3.9 1.7c-1 .8-1.7 1.2-1.7 2.8m0 3h.01"/>',
    refresh: '<path d="M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4"/>',
    open: '<path d="M12 19V5M5 12l7-7 7 7"/>',
    stop: '<rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor" stroke="none"/>',
    close: '<path d="M12 5v14m7-7-7 7-7-7"/>',
    close_small: '<path d="m6 6 12 12M18 6 6 18"/>',
    target: '<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="3"/><path d="M12 1v3m0 16v3M1 12h3m16 0h3"/>',
    trash: '<path d="M4 7h16M10 11v6m4-6v6M5 7l1 14h12l1-14M9 7V4h6v3"/>',
};
const availabilityIcons = { ready: 'observed', stale: 'stale', not_seen: 'not_seen', ambiguous: 'ambiguous' };
export const DISPLAY_MODE_OPTIONS = [
    ['auto', 'Auto'],
    ['mobile', 'Mobile'],
    ['panel', 'Panel'],
    ['desktop', 'Desktop'],
];
function iconMarkup(name) {
    if (!name)
        return '';
    const paths = icons[name];
    return paths
        ? `<svg class="ui-icon" data-icon="${escapeHtml(name)}" viewBox="0 0 24 24" aria-hidden="true" focusable="false">${paths}</svg>`
        : '';
}
export function renderIcon(name) { return iconMarkup(name); }
export function blindState(positionPercent, movementStatus) {
    const percentClosed = isPercent(positionPercent)
        ? positionPercent
        : null;
    if (movementStatus === 'opening')
        return { label: 'Opening', percentClosed };
    if (movementStatus === 'closing')
        return { label: 'Closing', percentClosed };
    if (percentClosed === null)
        return { label: 'Position unavailable', percentClosed: null };
    if (percentClosed === 0)
        return { label: 'Open', percentClosed };
    if (percentClosed === 100)
        return { label: 'Closed', percentClosed };
    return { label: 'Partially open', percentClosed };
}
export function movementStatusFromPositions(currentPercent, targetPercent) {
    const validCurrent = isPercent(currentPercent);
    const validTarget = isPercent(targetPercent);
    if (!validCurrent || !validTarget)
        return 'unavailable';
    if (currentPercent === targetPercent)
        return 'at_target';
    return targetPercent > currentPercent ? 'closing' : 'opening';
}
function isPercent(value) {
    return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 100;
}
export function renderBlindStatus({ positionPercent, movementStatus, compact = false, panelMode = false }) {
    const { label, percentClosed } = blindState(positionPercent, movementStatus);
    const variants = [compact && 'blind-state--compact', panelMode && 'blind-state--panel'].filter(Boolean).join(' ');
    return `<div class="blind-state${variants ? ` ${variants}` : ''}"><strong class="blind-state__label">${escapeHtml(label)}</strong>${percentClosed === null ? '' : `<span class="blind-state__percent">${percentClosed}% closed</span>`}</div>`;
}
export function renderPanelNotice({ label, details, tone = 'warn' }) {
    const safeTone = tone === 'bad' || tone === 'error' ? 'bad' : tone === 'good' || tone === 'success' ? 'good' : 'warn';
    const description = [label, ...details].filter(Boolean).join('. ');
    return `<p class="panel-notice panel-notice--${safeTone}" role="status" aria-label="${escapeHtml(description)}" title="${escapeHtml(description)}"><span class="panel-notice__marker" aria-hidden="true">!</span><span class="panel-notice__label">${escapeHtml(label)}</span></p>`;
}
export function renderBadge({ label, tone = '' }) {
    const variant = tone ? ` ${escapeHtml(tone)}` : '';
    return `<span class="badge${variant}">${escapeHtml(label)}</span>`;
}
export function renderStatusIndicator({ label, tone = '', status = 'unavailable' } = {}) {
    const icon = availabilityIcons[status] || 'unavailable';
    const toneClass = tone ? ` ${escapeHtml(tone)}` : '';
    const name = escapeHtml(label || 'Availability unavailable');
    return `<span class="status-indicator${toneClass}" role="img" aria-label="${name}" title="${name}">${iconMarkup(icon)}</span>`;
}
function renderDisplayModeToggle({ displayPreference = 'auto', themePreference = 'auto' } = {}) {
    const selectedIndex = DISPLAY_MODE_OPTIONS.findIndex(([value]) => value === displayPreference);
    const safeIndex = selectedIndex < 0 ? 0 : selectedIndex;
    const [, currentLabel] = DISPLAY_MODE_OPTIONS[safeIndex] ?? DISPLAY_MODE_OPTIONS[0];
    const accessibleName = `Display and theme settings. Current display: ${currentLabel}. Current theme: ${themePreference}.`;
    const modeIcons = { auto: 'display', mobile: 'mobile', panel: 'panel', desktop: 'desktop' };
    const modeChoices = DISPLAY_MODE_OPTIONS.map(([value, label]) => renderButton({
        label, accessibleLabel: label, title: label, action: `display:set:${value}`, icon: modeIcons[value], iconOnly: true,
        variant: 'display-menu-choice',
    }).replace('<button ', `<button role="menuitemradio" aria-checked="${value === DISPLAY_MODE_OPTIONS[safeIndex]?.[0]}" `)).join('');
    const themeChoices = [['auto', 'Automatic', 'display'], ['dark', 'Dark', 'moon'], ['light', 'Light', 'sun'], ['high-contrast', 'High contrast', 'contrast']].map(([value, label, icon]) => renderButton({
        label, accessibleLabel: `${label} theme`, title: `${label} theme`, action: `theme:set:${value}`, icon, iconOnly: true,
        variant: 'display-menu-choice',
    }).replace('<button ', `<button role="menuitemradio" aria-checked="${value === themePreference}" `)).join('');
    return `<div class="display-picker"><button type="button" id="display-mode-toggle" class="icon-button display-mode-button" data-action="display-menu-toggle" aria-label="${escapeHtml(accessibleName)}" title="${escapeHtml(accessibleName)}" aria-haspopup="menu" aria-expanded="false" aria-controls="display-menu">${iconMarkup('display')}</button><div class="display-menu" id="display-menu" role="menu" aria-label="Display and theme settings" hidden><div class="display-menu__section" aria-label="Display mode">${modeChoices}</div><div class="display-menu__section" aria-label="Theme">${themeChoices}</div></div></div>`;
}
export function renderDeviceCardHeader({ id, label, availabilityLabel, availabilityTone = '', availabilityStatus = 'unavailable', isSimulated = false, room, lastSeen = '' }) {
    const escapedId = escapeHtml(id);
    const escapedLabel = escapeHtml(label);
    const kind = isSimulated ? 'SIMULATED BLIND' : 'YOUR BLIND';
    return `<div class="card-top device-card-header"><div class="device-card-heading"><div class="subtle">${kind}</div><h3><button type="button" class="card-title" data-action="details:${escapedId}" aria-label="View details for ${escapedLabel}">${escapedLabel}</button></h3>${room ? `<div class="subtle room-label">${escapeHtml(room)}</div>` : ''}<div class="subtle">${escapeHtml(lastSeen)}</div></div>${renderStatusIndicator({ label: availabilityLabel, tone: availabilityTone, status: availabilityStatus })}</div>`;
}
export function renderButton({ label, action, type = 'button', variant = '', disabled = false, icon = null, iconOnly = false, accessibleLabel = label, title = '', }) {
    const classes = ['button', variant, iconOnly ? 'icon-action' : ''].filter(Boolean).map(escapeHtml).join(' ');
    const accessibility = iconOnly || accessibleLabel !== label ? ` aria-label="${escapeHtml(accessibleLabel)}"` : '';
    const tooltipLabel = title || (iconOnly ? accessibleLabel : '');
    const titleAttribute = tooltipLabel
        ? ` title="${escapeHtml(tooltipLabel)}"`
        : '';
    const iconContent = iconMarkup(icon);
    const labelContent = iconOnly ? '' : `<span class="button-label">${escapeHtml(label)}</span>`;
    const actionAttribute = action === undefined ? '' : ` data-action="${escapeHtml(action)}"`;
    return `<button type="${escapeHtml(type)}" class="${classes}"${actionAttribute}${accessibility}${titleAttribute}${disabled ? ' disabled' : ''}>${iconContent}${labelContent}</button>`;
}
export function renderIconButton({ label, action, icon, variant = '', disabled = false }) {
    return renderButton({
        label,
        accessibleLabel: label,
        title: label,
        action,
        variant: ['icon-button', variant].filter(Boolean).join(' '),
        disabled,
        icon,
        iconOnly: true,
    });
}
export function renderUtilityIconButton({ label, action, icon, variant = '', disabled = false }) {
    return renderIconButton({
        label,
        action,
        icon,
        variant: ['utility-icon-button', variant].filter(Boolean).join(' '),
        disabled,
    });
}
export function renderDismissButton({ label, action, variant = '', disabled = false }) {
    return renderUtilityIconButton({
        label,
        action,
        icon: 'close_small',
        variant: ['dismiss-button', variant].filter(Boolean).join(' '),
        disabled,
    });
}
export function renderServiceSummary(summary) {
    const action = renderButton({
        label: 'View service status',
        action: 'service',
        icon: 'service',
        variant: 'secondary service-status-action',
    });
    return `<div class="section-heading service-summary"><h2>Somaris service</h2>${action}</div><p class="subtle">${escapeHtml(summary)}</p>`;
}
export function renderDemoBanner() {
    const reset = renderButton({
        label: 'Reset demo',
        action: 'demo-reset',
        icon: 'reset',
        variant: 'secondary demo-banner__reset',
    });
    return `<section class="demo-banner" aria-label="Somaris demonstration"><div><p class="eyebrow">Somaris Demo</p><p>Explore a simulated smart home. Your changes only affect this demonstration.</p></div>${reset}</section>`;
}
export function renderIconLinkButton({ label, action, icon, disabled = false }) {
    return renderButton({
        label,
        accessibleLabel: label,
        title: label,
        action,
        variant: 'icon-link-button',
        disabled,
        icon,
        iconOnly: true,
    });
}
export function renderDestructiveButton({ label, action, disabled = false, icon = 'trash' }) {
    return renderButton({ label, action, variant: 'danger', disabled, icon });
}
export function renderMotorSpeedControl({ deviceId, busy = false, calibrationActive = false }) {
    const id = escapeHtml(deviceId);
    const disabled = busy || calibrationActive;
    return `<section class="panel device-settings"><h2>Motor speed</h2><p>Set the motor speed from 0% to 100%. SOMA controllers need firmware 2.2.0 or later. The current setting is not read by Somaris yet.</p><form data-motor-speed="${id}"><label class="field">Speed (%)<input name="motor_speed_percent" type="number" min="0" max="100" step="1" required placeholder="0–100"${disabled ? ' disabled' : ''}></label><button class="button"${disabled ? ' disabled' : ''}>Set speed</button></form></section>`;
}
export function renderCalibrationControls({ deviceId, active = false, busy = false }) {
    const id = String(deviceId ?? '');
    if (!active) {
        return renderButton({
            label: 'Start endpoint calibration',
            action: `command:calibration_start:${id}`,
            variant: 'secondary',
            disabled: busy,
        });
    }
    const controls = [
        ['Nudge open', 'calibration_nudge_open'],
        ['Set open limit', 'calibration_set_open_limit'],
        ['Nudge closed', 'calibration_nudge_closed'],
        ['Set closed limit', 'calibration_set_closed_limit'],
        ['Stop movement', 'stop'],
        ['End calibration', 'calibration_stop'],
    ];
    return `<div class="calibration-actions">${controls.map(([label, action]) => renderButton({
        label,
        action: `command:${action}:${id}`,
        variant: action === 'calibration_stop' ? 'danger' : 'secondary',
        disabled: busy,
    })).join('')}</div>`;
}
export function renderConfirmationPanel({ title, message, confirmLabel = 'Confirm', confirmAction, cancelAction, disabled = false, }) {
    return `<section class="confirmation-panel panel" role="group" aria-label="${escapeHtml(title)}"><div class="confirmation-copy"><h2>${escapeHtml(title)}</h2><p>${escapeHtml(message)}</p></div><div class="confirmation-actions">${renderButton({ label: 'Cancel', action: cancelAction, variant: 'secondary', disabled })}${renderDestructiveButton({ label: confirmLabel, action: confirmAction, disabled })}</div></section>`;
}
export function renderAddBlindButton({ variant = '' } = {}) {
    return renderIconButton({
        label: 'Add a blind',
        action: 'add',
        variant: ['add-icon-button', 'topbar-add', variant].filter(Boolean).join(' '),
        icon: 'add',
    });
}
export function renderRoomGroup({ room, cardsMarkup }) {
    return `<section class="room-group"><h3 class="room-heading">${escapeHtml(room)}</h3><div class="grid room-group__grid">${cardsMarkup}</div></section>`;
}
export function renderPositionPreset({ deviceId, blindLabel, position, selectedPosition, disabled = false }) {
    const label = `Move ${blindLabel} to ${position}% closed`;
    const encodedId = encodeURIComponent(deviceId);
    const markup = renderButton({
        label: `${position}%`,
        accessibleLabel: label,
        title: label,
        action: `position:${encodedId}:${position}`,
        variant: 'position-preset',
        disabled,
    });
    return markup
        .replace('<button ', `<button aria-pressed="${position === selectedPosition}" `)
        .replace(' data-action=', ` data-position-preset="${escapeHtml(encodedId)}:${position}" data-action=`);
}
export function renderPositionControl({ deviceId, blindLabel, positionClosedPercent, disabled = false }) {
    const id = escapeHtml(deviceId);
    const presets = [0, 25, 50, 75, 100]
        .map(position => renderPositionPreset({ deviceId, blindLabel, position, selectedPosition: positionClosedPercent, disabled }))
        .join('');
    return `<div class="position-control" role="group" aria-label="Set position for ${escapeHtml(blindLabel)}"><div class="position-control__label"><label for="position-${id}">Set position</label><span><output data-position-output="${id}">${escapeHtml(positionClosedPercent)}%</output> closed</span></div><input id="position-${id}" type="range" min="0" max="100" step="1" value="${escapeHtml(positionClosedPercent)}" data-position-input="${id}" aria-label="Target position for ${escapeHtml(blindLabel)} in percent closed"${disabled ? ' disabled' : ''}><div class="position-presets" role="group" aria-label="Position presets for ${escapeHtml(blindLabel)}">${presets}</div></div>`;
}
export function renderTopbarActions({ blindCount = 0, panelMode = false, demoMode = false, displayPreference = 'auto', serviceLabel = 'Connecting', serviceTone = '', themePreference = 'auto', route = 'home', } = {}) {
    const count = Number.isInteger(blindCount) && blindCount >= 0 ? blindCount : 0;
    const countLabel = `${count} ${count === 1 ? 'blind' : 'blinds'}`;
    const status = `<span id="service-status" class="service-status"><span class="status-dot${serviceTone ? ` ${escapeHtml(serviceTone)}` : ''}"></span> ${escapeHtml(serviceLabel)}</span>`;
    const panelCount = panelMode ? `<span class="topbar-device-count">${escapeHtml(countLabel)}</span>` : '';
    const displayToggle = renderDisplayModeToggle({ displayPreference, themePreference });
    const refreshButton = renderIconButton({ label: 'Refresh', action: 'refresh', icon: 'refresh', variant: 'topbar-icon-button' });
    const serviceButton = !panelMode ? renderIconButton({ label: 'Service status', action: 'service', icon: 'service', variant: 'topbar-icon-button' }) : '';
    const demoReset = panelMode && demoMode ? renderIconButton({ label: 'Reset demo', action: 'demo-reset', icon: 'reset', variant: 'topbar-icon-button' }) : '';
    return `${status}${panelCount}${serviceButton}${demoReset}${renderAddBlindButton()}${displayToggle}${refreshButton}`;
}
export function renderMobileTabs(route = 'home') {
    const active = route.startsWith('details/') || route === 'add' ? 'home' : route;
    const tabs = [
        ['home', 'Blinds', 'home'], ['activity', 'Activity', 'activity'], ['service', 'Service status', 'service'],
    ];
    return `<nav class="mobile-tabs" aria-label="Main navigation">${tabs.map(([id, label, icon]) => `<a class="mobile-tab" href="#${id}" aria-label="${label}" title="${label}"${active === id ? ' aria-current="page"' : ''}>${iconMarkup(icon)}</a>`).join('')}</nav>`;
}
export function renderBlindActions({ deviceId, blindLabel, panelMode = false, disabled = false, busy = false, movementDisabled = false, supportsPosition = false, positionClosedPercent = 100 }) {
    const actions = [
        ['open', 'Open', ''],
        ['stop', 'Stop', 'secondary'],
        ['close', 'Close', 'secondary'],
    ];
    const disabledPosition = disabled || movementDisabled || busy;
    const positionControls = supportsPosition && !panelMode
        ? renderPositionControl({ deviceId, blindLabel, positionClosedPercent, disabled: disabledPosition })
        : '';
    return `<div class="controls" role="group" aria-label="${escapeHtml(blindLabel)} controls">${actions.map(([action, label, variant]) => {
        const accessibleLabel = `${label} ${blindLabel}`;
        return renderButton({
            label,
            accessibleLabel,
            action: `command:${action}:${deviceId}`,
            variant: [variant, panelMode ? 'panel-target' : ''].filter(Boolean).join(' '),
            disabled: disabled || (movementDisabled && action !== 'stop') || (busy && action !== 'stop'),
            icon: action,
            iconOnly: panelMode,
            title: panelMode ? accessibleLabel : '',
        });
    }).join('')}${positionControls}</div>`;
}
export function renderBatchFeedback(batch) {
    return renderBatchFeedbackMarkup(batch, escapeHtml, renderButton);
}
export function renderBatchControls({ blindCount = 0, eligibleCount = 0, disabled = false, busy = false, panelMode = false, batch = null } = {}) {
    if (blindCount < 1 && !batch)
        return '';
    const unavailable = disabled || busy || eligibleCount < 1;
    const description = eligibleCount === 1
        ? 'Open or close the available blind.'
        : eligibleCount > 1
            ? `Open or close all ${eligibleCount} available blinds in sequence.`
            : 'No enrolled blinds are currently ready to move.';
    const panelVariant = panelMode ? ' all-blinds-controls--panel' : '';
    const controls = blindCount > 0
        ? `<section class="all-blinds-controls${panelVariant}" aria-label="All blinds controls"${busy ? ' aria-busy="true"' : ''}><div class="all-blinds-copy"><h2>All blinds</h2><p>${escapeHtml(description)}</p></div><div class="all-blinds-actions">${renderButton({ label: 'Open all', accessibleLabel: 'Open all blinds', action: 'batch:open', icon: 'open', iconOnly: panelMode, title: panelMode ? 'Open all blinds' : '', variant: panelMode ? 'panel-target' : '', disabled: unavailable })}${renderButton({ label: 'Close all', accessibleLabel: 'Close all blinds', action: 'batch:close', icon: 'close', iconOnly: panelMode, title: panelMode ? 'Close all blinds' : '', variant: panelMode ? 'secondary panel-target' : 'secondary', disabled: unavailable })}</div>${renderBatchFeedback(batch)}</section>`
        : '';
    return `${controls}${blindCount < 1 ? renderBatchFeedback(batch) : ''}`;
}
export function renderBlindCard({ id, label, isSimulated, room, lastSeen, availabilityLabel, availabilityTone = '', availabilityStatus = 'unavailable', positionPercent, movementStatus, blindLines = 0, controlsMarkup, messageMarkup = '', footerMarkup = '', panelMode = false, supportsPosition = false, }) {
    const slats = '<i></i>'.repeat(Math.max(0, Math.min(11, blindLines)));
    const fill = `<span class="blind-art__fill" style="--blind-fill:${Math.max(0, Math.min(100, positionPercent ?? 0))}%">${slats}</span>`;
    const cardVariant = panelMode ? ' card--panel' : '';
    const displayVariant = panelMode ? ' blind-display--panel' : '';
    const header = renderDeviceCardHeader({ id, label, availabilityLabel, availabilityTone, availabilityStatus, isSimulated, room, lastSeen });
    const status = renderBlindStatus({ positionPercent, movementStatus, panelMode });
    const graphic = supportsPosition && !panelMode
        ? `<div class="blind-art blind-art--interactive" data-position-graphic="${escapeHtml(id)}" role="slider" tabindex="0" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${escapeHtml(positionPercent ?? 0)}" aria-label="Blind position for ${escapeHtml(label)}, percent closed" data-movement="${escapeHtml(movementStatus)}">${fill}</div>`
        : `<div class="blind-art" data-movement="${escapeHtml(movementStatus)}" aria-hidden="true">${fill}</div>`;
    return `<article class="card${cardVariant}" data-device-card="${escapeHtml(id)}" data-room="${escapeHtml(room || '')}">${header}<div class="blind-display${displayVariant}">${status}${graphic}</div>${controlsMarkup}<div class="card-messages" data-card-messages="${escapeHtml(id)}">${messageMarkup}</div><div class="card-foot">${footerMarkup}</div></article>`;
}
