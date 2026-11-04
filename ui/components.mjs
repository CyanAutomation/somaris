export const escapeHtml = value => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;',
}[character]));

const icons = {
  add:'<path d="M12 5v14M5 12h14"/>',
  open:'<path d="M12 19V5M5 12l7-7 7 7"/>',
  stop:'<rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor" stroke="none"/>',
  close:'<path d="M12 5v14m7-7-7 7-7-7"/>',
};

export function iconMarkup(name) {
  const paths = icons[name];
  return paths
    ? `<svg class="ui-icon" data-icon="${escapeHtml(name)}" viewBox="0 0 24 24" aria-hidden="true" focusable="false">${paths}</svg>`
    : '';
}

export function blindState(positionPercent, movementStatus) {
  const percentClosed = Number.isInteger(positionPercent) && positionPercent >= 0 && positionPercent <= 100
    ? positionPercent
    : null;
  if (movementStatus === 'opening') return {label:'Opening', percentClosed};
  if (movementStatus === 'closing') return {label:'Closing', percentClosed};
  if (percentClosed === null) return {label:'Position unavailable', percentClosed:null};
  if (percentClosed === 0) return {label:'Open', percentClosed};
  if (percentClosed === 100) return {label:'Closed', percentClosed};
  return {label:'Partially open', percentClosed};
}

export function movementStatusFromPositions(currentPercent, targetPercent) {
  const validCurrent = Number.isInteger(currentPercent) && currentPercent >= 0 && currentPercent <= 100;
  const validTarget = Number.isInteger(targetPercent) && targetPercent >= 0 && targetPercent <= 100;
  if (!validCurrent || !validTarget) return 'unavailable';
  if (currentPercent === targetPercent) return 'at_target';
  return targetPercent > currentPercent ? 'closing' : 'opening';
}

export function renderBlindStatus({positionPercent, movementStatus, compact = false, panelMode = false}) {
  const {label, percentClosed} = blindState(positionPercent, movementStatus);
  const variants = [compact && 'blind-state--compact', panelMode && 'blind-state--panel'].filter(Boolean).join(' ');
  return `<div class="blind-state${variants ? ` ${variants}` : ''}"><strong class="blind-state__label">${escapeHtml(label)}</strong>${percentClosed === null ? '' : `<span class="blind-state__percent">${percentClosed}% closed</span>`}</div>`;
}

export function renderBadge({label, tone = ''}) {
  const variant = tone ? ` ${escapeHtml(tone)}` : '';
  return `<span class="badge${variant}">${escapeHtml(label)}</span>`;
}

export function renderDeviceCardHeader({id, label, availabilityLabel, availabilityTone = '', isSimulated = false, lastSeen = ''}) {
  const escapedId = escapeHtml(id);
  const escapedLabel = escapeHtml(label);
  const kind = isSimulated ? 'SIMULATED BLIND' : 'YOUR BLIND';
  return `<div class="card-top device-card-header"><div class="device-card-heading"><div class="subtle">${kind}</div><h3><button type="button" class="card-title" data-action="details:${escapedId}" aria-label="View details for ${escapedLabel}">${escapedLabel}</button></h3><div class="subtle">${escapeHtml(lastSeen)}</div></div>${renderBadge({label:availabilityLabel, tone:availabilityTone})}</div>`;
}

export function renderButton({
  label,
  action,
  variant = '',
  disabled = false,
  icon = null,
  iconOnly = false,
  accessibleLabel = label,
  title = '',
}) {
  const classes = ['button', variant, iconOnly ? 'icon-action' : ''].filter(Boolean).map(escapeHtml).join(' ');
  const accessibility = iconOnly ? ` aria-label="${escapeHtml(accessibleLabel)}"` : '';
  const tooltipLabel = title || (iconOnly ? accessibleLabel : '');
  const titleAttribute = tooltipLabel
    ? ` title="${escapeHtml(tooltipLabel)}"`
    : '';
  const iconContent = iconMarkup(icon);
  const labelContent = iconOnly ? '' : `<span class="button-label">${escapeHtml(label)}</span>`;
  return `<button type="button" class="${classes}" data-action="${escapeHtml(action)}"${accessibility}${titleAttribute}${disabled ? ' disabled' : ''}>${iconContent}${labelContent}</button>`;
}

export function renderAddBlindButton({variant = ''} = {}) {
  return renderButton({
    label:'Add a blind',
    accessibleLabel:'Add a blind',
    title:'Add a blind',
    action:'add',
    variant:['add-icon-button', variant].filter(Boolean).join(' '),
    icon:'add',
    iconOnly:true,
  });
}

export function renderBlindActions({deviceId, blindLabel, panelMode = false, disabled = false, busy = false}) {
  const actions = [
    ['open', 'Open', ''],
    ['stop', 'Stop', 'secondary'],
    ['close', 'Close', 'secondary'],
  ];
  return `<div class="controls" role="group" aria-label="${escapeHtml(blindLabel)} controls">${actions.map(([action, label, variant]) => {
    const accessibleLabel = `${label} ${blindLabel}`;
    return renderButton({
      label,
      accessibleLabel,
      action:`command:${action}:${deviceId}`,
      variant,
      disabled:disabled || (busy && action !== 'stop'),
      icon:action,
      iconOnly:panelMode,
      title:panelMode ? accessibleLabel : '',
    });
  }).join('')}</div>`;
}

export function renderBlindCard({
  id,
  label,
  isSimulated,
  lastSeen,
  availabilityLabel,
  availabilityTone = '',
  positionPercent,
  movementStatus,
  blindLines = 0,
  controlsMarkup,
  messageMarkup = '',
  footerMarkup = '',
  panelMode = false,
}) {
  const slats = '<i></i>'.repeat(Math.max(0, Math.min(11, blindLines)));
  const cardVariant = panelMode ? ' card--panel' : '';
  const displayVariant = panelMode ? ' blind-display--panel' : '';
  const header = renderDeviceCardHeader({id, label, availabilityLabel, availabilityTone, isSimulated, lastSeen});
  const status = renderBlindStatus({positionPercent, movementStatus, panelMode});
  return `<article class="card${cardVariant}">${header}<div class="blind-display${displayVariant}">${status}<div class="blind-art" aria-hidden="true">${slats}</div></div>${controlsMarkup}${messageMarkup}<div class="card-foot">${footerMarkup}</div></article>`;
}

export function renderDeviceList({count, panelMode = false, addButton = '', cardsMarkup = '', emptyMarkup = '', title = 'Your blinds'}) {
  const titleContent = panelMode ? '' : `<h2>${escapeHtml(title)}</h2>`;
  const addContent = panelMode ? addButton : '';
  const countLabel = `${count} ${count === 1 ? 'blind' : 'blinds'}`;
  const content = cardsMarkup ? `<div class="grid">${cardsMarkup}</div>` : emptyMarkup;
  return `<section class="device-list${panelMode ? ' device-list--panel' : ''}" aria-label="${escapeHtml(title)}"><div class="section-heading device-list-heading">${titleContent}<span class="count">${countLabel}</span>${addContent}</div>${content}</section>`;
}

export function renderHomeSupportingSections({panelMode = false, activityMarkup = '', serviceMarkup = ''}) {
  return panelMode ? '' : `${activityMarkup}${serviceMarkup}`;
}

export function renderInfoItem({label, value, contentMarkup}) {
  const content = contentMarkup === undefined ? `<strong>${escapeHtml(value)}</strong>` : contentMarkup;
  return `<div class="info-item"><span class="meta-label subtle">${escapeHtml(label)}</span>${content}</div>`;
}

export function renderInfoGrid(items) {
  return `<div class="info-grid">${items.map(renderInfoItem).join('')}</div>`;
}
