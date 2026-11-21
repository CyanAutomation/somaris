import type {
  Availability,
  BatchResponse,
  BatchState,
  BatteryReading,
  DisplayPreference,
  VisualTone,
} from './types.mjs';
import {escapeHtml} from './escape.mjs';
import {renderBatchFeedbackMarkup} from './batch-feedback.mjs';
import type {BatchFeedbackView} from './batch-feedback.mjs';
import {renderInfoGrid} from './page-components.mjs';

export {escapeHtml};
export {renderDeviceList, renderHomeSupportingSections, renderInfoGrid} from './page-components.mjs';
export {renderSnackbar, renderSnackbarRegion} from './snackbar-components.mjs';

const icons: Record<string, string> = {
  add:'<path d="M12 5v14M5 12h14"/>',
  activity:'<path d="M3 12h4l3-8 4 16 3-8h4"/>',
  service:'<path d="M4 5h16v14H4z"/><path d="M8 9h8m-8 4h5"/>',
  settings:'<path d="M10 2h4l.5 2.2 1.5.8 2.1-.8 2 3.5-1.7 1.4v1.8l1.7 1.4-2 3.5-2.1-.8-1.5.8L14 18h-4l-.5-2.2-1.5-.8-2.1.8-2-3.5 1.7-1.4V9.1L3.9 7.7l2-3.5 2.1.8 1.5-.8L10 2Z"/><circle cx="12" cy="10" r="2.1"/>',
  sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.42 1.42m11.3 11.3 1.42 1.42M2 12h2m16 0h2M4.93 19.07l1.42-1.42m11.3-11.3 1.42-1.42"/>',
  moon:'<path d="M20.5 15.5A8.5 8.5 0 0 1 8.5 3.5 8.5 8.5 0 1 0 20.5 15.5Z"/>',
  contrast:'<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 0 0 18z"/>',
  mobile:'<rect x="6" y="2.5" width="12" height="19" rx="2"/><path d="M10 18.5h4"/>',
  desktop:'<rect x="3" y="4" width="18" height="13" rx="1.5"/><path d="M8 21h8m-4-4v4"/>',
  panel:'<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 3v18M3 12h18"/>',
  chevron:'<path d="m7 10 5 5 5-5"/>',
  undo:'<path d="M9 14 4 9l5-5"/><path d="M4 9h9a7 7 0 0 1 0 14h-2" transform="translate(0 -2)"/>',
  reset:'<path d="M4 7v5h5M5.5 12a7 7 0 1 0 2.1-5"/><path d="M12 8v4l3 2"/>',
  spinner:'<path d="M20 12a8 8 0 1 1-2.34-5.66"/>',
  check:'<circle cx="12" cy="12" r="9"/><path d="m8 12 2.5 2.5L16.5 9"/>',
  warning:'<path d="M12 3 22 20H2L12 3Z"/><path d="M12 9v5m0 3h.01"/>',
  home:'<path d="m3 11 9-8 9 8v10h-6v-6H9v6H3z"/>',
  position0:'<rect x="4" y="3" width="16" height="18" rx="1.5"/>',
  position1:'<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 8h14"/>',
  position2:'<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 8h14m-14 5h14"/>',
  position3:'<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 7h14m-14 5h14m-14 5h14"/>',
  position4:'<rect x="4" y="3" width="16" height="18" rx="1.5"/><path d="M5 6h14m-14 4h14m-14 4h14m-14 4h14"/>',
  back:'<path d="M19 12H5m7 7-7-7 7-7"/>',
  display:'<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/>',
  info:'<circle cx="12" cy="12" r="9"/><path d="M12 11v5m0-8h.01"/>',
  battery:'<rect x="3" y="7" width="17" height="10" rx="2"/><path d="M22 10v4M7 10v4m4-4v4m4-4v4"/>',
  chevron_right:'<path d="m9 18 6-6-6-6"/>',
  observed:'<circle cx="12" cy="12" r="8.5"/><path d="m8.5 12 2.3 2.3 4.8-5"/>',
  clock:'<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3 2"/>',
  stale:'<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3 2"/>',
  not_seen:'<circle cx="12" cy="12" r="8.5"/><path d="m9 9 6 6m0-6-6 6"/>',
  ambiguous:'<path d="M12 3.5 22 20H2L12 3.5Z"/><path d="M12 9v5m0 3h.01"/>',
  unavailable:'<circle cx="12" cy="12" r="8.5"/><path d="M9.8 9a2.3 2.3 0 1 1 3.9 1.7c-1 .8-1.7 1.2-1.7 2.8m0 3h.01"/>',
  refresh:'<path d="M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4"/>',
  open:'<path d="M12 19V5M5 12l7-7 7 7"/>',
  stop:'<rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor" stroke="none"/>',
  close:'<path d="M12 5v14m7-7-7 7-7-7"/>',
  close_small:'<path d="m6 6 12 12M18 6 6 18"/>',
  target:'<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="3"/><path d="M12 1v3m0 16v3M1 12h3m16 0h3"/>',
  trash:'<path d="M4 7h16M10 11v6m4-6v6M5 7l1 14h12l1-14M9 7V4h6v3"/>',
};
type IconName = keyof typeof icons;
const availabilityIcons: Partial<Record<Availability, IconName>> = {ready:'observed', stale:'stale', not_seen:'not_seen', ambiguous:'ambiguous'};

export const DISPLAY_MODE_OPTIONS = [
  ['auto', 'Auto'],
  ['mobile', 'Mobile'],
  ['panel', 'Compact'],
  ['desktop', 'Desktop'],
] as const satisfies readonly (readonly [DisplayPreference, string])[];

interface BlindStatusOptions {
  positionPercent: number | null | undefined;
  movementStatus: MovementStatus;
  compact?: boolean;
  panelMode?: boolean;
}
interface PanelNoticeOptions {
  label: string;
  details: string[];
  tone?: VisualTone;
}

type MovementStatus = 'opening' | 'closing' | 'stopped' | 'at_target' | 'unavailable' | 'stop_queued' | 'stopping';

interface BadgeOptions {
  label: string;
  tone?: VisualTone;
}

interface StatusIndicatorOptions {
  label?: string;
  tone?: VisualTone;
  status?: Availability;
}

interface DeviceCardHeaderOptions {
  id: string;
  label: string;
  availabilityLabel: string;
  availabilityTone?: VisualTone;
  availabilityStatus?: Availability;
  isSimulated?: boolean;
  room?: string;
  showRoom?: boolean;
  lastSeen?: string;
  panelMode?: boolean;
}

interface RoomPickerOptions {
  id: string;
  rooms: string[];
  selectedRoom?: string | null;
}

interface ButtonOptions {
  label: string;
  action?: string;
  type?: 'button' | 'submit';
  variant?: string;
  disabled?: boolean;
  icon?: string | null;
  iconOnly?: boolean;
  accessibleLabel?: string;
  title?: string;
}

interface ActivitySectionOptions {
  count: number;
  expanded: boolean;
  listMarkup: string;
}

interface TopbarActionsOptions {
  blindCount?: number;
  panelMode?: boolean;
  demoMode?: boolean;
  displayPreference?: string;
  serviceLabel?: string;
  serviceShortLabel?: string;
  serviceTone?: VisualTone;
  themePreference?: string;
  route?: string;
}

interface BlindActionsOptions {
  deviceId: string;
  blindLabel: string;
  panelMode?: boolean;
  disabled?: boolean;
  busy?: boolean;
  movementDisabled?: boolean;
  supportsPosition?: boolean;
  positionClosedPercent?: number;
  currentClosedPercent?: number | null;
  currentPositionFresh?: boolean;
}

interface IconButtonOptions {
  label: string;
  accessibleLabel?: string;
  title?: string;
  action: string;
  icon: string;
  variant?: string;
  disabled?: boolean;
}

interface BlindCardOptions extends DeviceCardHeaderOptions, BlindStatusOptions {
  blindLines?: number;
  controlsMarkup: string;
  messageMarkup?: string;
  footerMarkup?: string;
  supportsPosition?: boolean;
  showPositionCue?: boolean;
}

interface BatchControlsOptions {
  blindCount?: number;
  eligibleCount?: number;
  disabled?: boolean;
  busy?: boolean;
  panelMode?: boolean;
  batch?: BatchState | BatchResponse | null;
}

function iconMarkup(name: string | null | undefined): string {
  if (!name) return '';
  const paths = icons[name];
  return paths
    ? `<svg class="ui-icon" data-icon="${escapeHtml(name)}" viewBox="0 0 24 24" aria-hidden="true" focusable="false">${paths}</svg>`
    : '';
}
export function renderIcon(name: string): string { return iconMarkup(name); }

export function blindState(positionPercent: number | null | undefined, movementStatus: MovementStatus): {label: string; percentOpen: number | null} {
  const percentClosed = isPercent(positionPercent)
    ? positionPercent
    : null;
  const percentOpen=percentClosed===null?null:100-percentClosed;
  if (movementStatus === 'opening') return {label:'Opening…', percentOpen};
  if (movementStatus === 'closing') return {label:'Closing…', percentOpen};
  if (movementStatus === 'stop_queued') return {label:'Stop queued', percentOpen};
  if (movementStatus === 'stopping') return {label:'Stopping', percentOpen};
  if (percentOpen === null) return {label:'Position unavailable', percentOpen:null};
  if (percentOpen === 100) return {label:'Open', percentOpen};
  if (percentOpen === 0) return {label:'Closed', percentOpen};
  return {label:'Partially open', percentOpen};
}

export function movementStatusFromPositions(currentPercent: number | null | undefined, targetPercent: number | null | undefined): MovementStatus {
  const validCurrent = isPercent(currentPercent);
  const validTarget = isPercent(targetPercent);
  if (!validCurrent || !validTarget) return 'unavailable';
  if (currentPercent === targetPercent) return 'at_target';
  return targetPercent > currentPercent ? 'closing' : 'opening';
}

function isPercent(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 100;
}

export function renderBlindStatus({positionPercent, movementStatus, compact = false, panelMode = false}: BlindStatusOptions): string {
  const {label, percentOpen} = blindState(positionPercent, movementStatus);
  const variants = [compact && 'blind-state--compact', panelMode && 'blind-state--panel'].filter(Boolean).join(' ');
  const moving=movementStatus==='opening'||movementStatus==='closing';
  const progress=percentOpen!==null&&moving?`<div class="blind-progress" role="progressbar" aria-label="${escapeHtml(label)} ${percentOpen}% open" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${percentOpen}"><span style="--progress:${percentOpen}%"></span></div>`:'';
  const percent=percentOpen===null?'':`<span class="blind-state__percent">${percentOpen}% open</span>`;
  return `<div class="blind-state${variants ? ` ${variants}` : ''}"><strong class="blind-state__label">${escapeHtml(label)}${moving&&percentOpen!==null?` ${percentOpen}%`:''}</strong>${percent}${progress}</div>`;
}

export function renderPanelNotice({label, details, tone = 'warn'}: PanelNoticeOptions): string {
  const safeTone = tone === 'bad' || tone === 'error' ? 'bad' : tone === 'good' || tone === 'success' ? 'good' : 'warn';
  const description = [label, ...details].filter(Boolean).join('. ');
  return `<p class="panel-notice panel-notice--${safeTone}" role="status" aria-label="${escapeHtml(description)}" title="${escapeHtml(description)}"><span class="panel-notice__marker" aria-hidden="true">!</span><span class="panel-notice__label">${escapeHtml(label)}</span></p>`;
}

export function renderBadge({label, tone = ''}: BadgeOptions): string {
  const variant = tone ? ` ${escapeHtml(tone)}` : '';
  return `<span class="badge${variant}">${escapeHtml(label)}</span>`;
}

export function renderStatusIndicator({label, tone = '', status = 'unavailable'}: StatusIndicatorOptions = {}): string {
  const icon = availabilityIcons[status] || 'unavailable';
  const toneClass = tone ? ` ${escapeHtml(tone)}` : '';
  const name = escapeHtml(label || 'Availability unavailable');
  return `<span class="status-indicator${toneClass}" role="img" aria-label="${name}" title="${name}">${iconMarkup(icon)}</span>`;
}

export function relativeTimeLabel(date: string | null | undefined, now = Date.now()): string {
  if (!date || !Number.isFinite(Date.parse(date))) return 'Time unavailable';
  const timestamp=Date.parse(date);
  const seconds=Math.max(0,Math.floor((now-timestamp)/1000));
  return seconds<60?'just now':seconds<3600?`${Math.floor(seconds/60)} min ago`:seconds<86400?`${Math.floor(seconds/3600)} h ago`:`${Math.floor(seconds/86400)} days ago`;
}

export function renderRelativeTime(date: string | null | undefined, now = Date.now()): string {
  if (!date || !Number.isFinite(Date.parse(date))) return '<time>Time unavailable</time>';
  const timestamp=Date.parse(date);
  const label=relativeTimeLabel(date,now);
  const absolute=new Date(timestamp).toLocaleString();
  return `<time datetime="${escapeHtml(new Date(timestamp).toISOString())}" title="${escapeHtml(absolute)}">${label}</time>`;
}

export function renderBatteryReading({battery,now=Date.now()}: {battery:BatteryReading|null|undefined;now?:number}): string {
  const percentage=battery?.percent_estimate;
  if(typeof percentage!=='number'||!Number.isInteger(percentage)||percentage<0||percentage>100)return '<span class="battery-reading subtle">Battery unavailable</span>';
  const reading=battery?.observed_at?`<span class="battery-reading__age">Last reading ${renderRelativeTime(battery.observed_at,now)}</span>`:'';
  return `<div class="battery-reading" aria-label="Estimated battery ${percentage}%"><span class="battery-reading__icon">${iconMarkup('battery')}</span><span class="battery-reading__value">~${percentage}%</span>${reading}</div>`;
}

function renderLayoutControl({displayPreference='auto'}:{displayPreference?:string}={}):string {
  const selectedIndex = DISPLAY_MODE_OPTIONS.findIndex(([value]) => value === displayPreference);
  const safeIndex = selectedIndex < 0 ? 0 : selectedIndex;
  const [currentPreference, currentLabel] = DISPLAY_MODE_OPTIONS[safeIndex] ?? DISPLAY_MODE_OPTIONS[0];
  const modeIcons:Record<string,string>={auto:'display',mobile:'mobile',panel:'panel',desktop:'desktop'};
  const label=`Layout: ${currentLabel}`;
  return renderIconButton({label,action:'layout-cycle',icon:modeIcons[currentPreference]||'display',variant:'topbar-action topbar-icon-button layout-mode-toggle'}).replace('<button ',`<button id="layout-mode-toggle" data-layout-mode="${escapeHtml(currentPreference)}" `);
}

function renderThemeControl({themePreference='auto'}:{themePreference?:string}={}):string {
  const isDark=themePreference==='dark';
  const nextLabel=isDark?'Light':'Dark';
  const icons:Record<string,string>={auto:'display',dark:'moon',light:'sun','high-contrast':'contrast'};
  const label=`Switch to ${nextLabel.toLowerCase()} theme`;
  return renderIconButton({label,action:'theme-toggle',icon:icons[themePreference]||'display',variant:'topbar-action topbar-icon-button theme-toggle'}).replace('<button ',`<button id="theme-toggle" data-theme-preference="${escapeHtml(themePreference)}" `);
}

export function renderThemePreferenceControl(themePreference='auto'):string {
  const choices=[['auto','Automatic'],['dark','Dark'],['light','Light'],['high-contrast','High contrast']] as const;
  const selected=choices.some(([value])=>value===themePreference)?themePreference:'auto';
  const options=choices.map(([value,label])=>`<option value="${value}"${value===selected?' selected':''}>${label}</option>`).join('');
  return `<section class="panel appearance-settings"><h2>Appearance</h2><label class="field" for="theme-preference">Theme<select id="theme-preference" name="theme" aria-label="Theme" data-theme-preference>${options}</select></label><p class="subtle">The header theme button switches directly between light and dark. Automatic and high contrast are available here.</p></section>`;
}

export function renderFreshnessIndicator(label: string): string {
  const name=escapeHtml(label || 'Last heard time unavailable');
  return `<span class="freshness-indicator" role="img" aria-label="${name}" title="${name}">${iconMarkup('clock')}</span>`;
}

function cardDisplayName(label:string, room?:string):string {
  if(!room||label.slice(0,room.length).toLowerCase()!==room.toLowerCase())return label;
  const suffix=label.slice(room.length);
  const match=suffix.match(/^\s*(?:[:—–-])\s*(.+)$/);
  return match?.[1]?.trim()||label;
}

export function renderDeviceCardHeader({id, label, availabilityLabel, availabilityTone = '', availabilityStatus = 'unavailable', room, showRoom = false, lastSeen = '', panelMode = false}: DeviceCardHeaderOptions): string {
  const escapedId = escapeHtml(id);
  const escapedLabel = escapeHtml(label);
  const title=escapeHtml(cardDisplayName(label,room));
  const freshness=lastSeen ? panelMode ? renderFreshnessIndicator(lastSeen) : `<div class="subtle last-seen">${escapeHtml(lastSeen)}</div>` : '';
  const roomLabel=showRoom?`<span class="room-label">${escapeHtml(room||'Unassigned')}</span>`:'';
  return `<div class="card-top device-card-header"><div class="device-card-heading">${roomLabel}<h3><button type="button" class="card-title" data-action="details:${escapedId}" aria-label="View details for ${escapedLabel}"><span>${title}</span><span class="card-title-chevron">${iconMarkup('chevron_right')}</span></button></h3>${!panelMode&&freshness ? freshness : ''}</div><div class="device-card-indicators">${panelMode&&freshness ? freshness : ''}${renderStatusIndicator({label:availabilityLabel, tone:availabilityTone, status:availabilityStatus})}</div></div>`;
}

export function renderButton({
  label,
  action,
  type = 'button',
  variant = '',
  disabled = false,
  icon = null,
  iconOnly = false,
  accessibleLabel,
  title = '',
}: ButtonOptions): string {
  const classes = ['button', variant, iconOnly ? 'icon-action' : ''].filter(Boolean).map(escapeHtml).join(' ');
  const a11yName=accessibleLabel||label;
  const accessibility = iconOnly || accessibleLabel ? ` aria-label="${escapeHtml(a11yName)}"` : '';
  const tooltipLabel = title || (iconOnly ? a11yName : '');
  const titleAttribute = tooltipLabel
    ? ` title="${escapeHtml(tooltipLabel)}"`
    : '';
  const tooltipAttribute = tooltipLabel
    ? ` data-tooltip="${escapeHtml(tooltipLabel)}"`
    : '';
  const iconContent = iconMarkup(icon);
  const labelContent = iconOnly ? '' : `<span class="button-label">${escapeHtml(label)}</span>`;
  const actionAttribute = action === undefined ? '' : ` data-action="${escapeHtml(action)}"`;
  return `<button type="${escapeHtml(type)}" class="${classes}"${actionAttribute}${accessibility}${titleAttribute}${tooltipAttribute}${disabled ? ' disabled' : ''}>${iconContent}${labelContent}</button>`;
}

export function renderIconButton({label, accessibleLabel, title, action, icon, variant = '', disabled = false}: IconButtonOptions): string {
  return renderButton({
    label,
    accessibleLabel:accessibleLabel||label,
    title:title||label,
    action,
    variant:['icon-button', variant].filter(Boolean).join(' '),
    disabled,
    icon,
    iconOnly:true,
  });
}

export function renderUtilityIconButton({label, action, icon, variant = '', disabled = false}: IconButtonOptions): string {
  return renderIconButton({
    label,
    action,
    icon,
    variant:['utility-icon-button',variant].filter(Boolean).join(' '),
    disabled,
  });
}

export function renderDismissButton({label, action, variant = '', disabled = false}: {label:string; action:string; variant?:string; disabled?:boolean}): string {
  return renderUtilityIconButton({
    label,
    action,
    icon:'close_small',
    variant:['dismiss-button',variant].filter(Boolean).join(' '),
    disabled,
  });
}

export function renderServiceSummary(summary: string): string {
  const action=renderButton({
    label:'Open settings',
    accessibleLabel:'Open settings',
    action:'settings',
    icon:'settings',
    variant:'secondary service-status-action',
  });
  return `<div class="section-heading service-summary"><h2>Somaris service</h2>${action}</div><p class="subtle">${escapeHtml(summary)}</p>`;
}

export function renderServiceHealthSummary({summary,fields,notUsedMarkup=''}:{summary:string;fields:{label:string;value:string|number}[];notUsedMarkup?:string}):string {
  const unused=notUsedMarkup?`<details class="service-unused"><summary>Not used in demo</summary>${notUsedMarkup}</details>`:'';
  return `<section class="service-health"><h2 class="service-health__summary">${escapeHtml(summary)}</h2><div class="service-health__details">${renderInfoGrid(fields)}</div>${unused}</section>`;
}

export function renderSettingsPage({themePreference='auto',serviceSummary,serviceFields,notUsedMarkup='',simulatorMarkup=''}:{
  themePreference?:string;
  serviceSummary:string;
  serviceFields:{label:string;value:string|number}[];
  notUsedMarkup?:string;
  simulatorMarkup?:string;
}):string {
  return `${renderBackButton()}<h1 class="page-title">Settings</h1>${renderThemePreferenceControl(themePreference)}<section class="settings-service"><h2>Service status</h2>${renderServiceHealthSummary({summary:serviceSummary,fields:serviceFields,notUsedMarkup})}${simulatorMarkup}</section>`;
}

export function renderActivitySection({count,expanded,listMarkup}:ActivitySectionOptions):string {
  const collapsed=!expanded;
  const countMarkup=count>0?`<span class="count">${count} changes</span>`:'';
  const label=collapsed?'Show':'Hide';
  const toggle=renderButton({label,accessibleLabel:`${collapsed?'Show':'Hide'} recent activity`,action:'activity-toggle',icon:'chevron',variant:`utility-button activity-toggle${collapsed?' is-collapsed':''}`}).replace('<button ',`<button aria-expanded="${!collapsed}" aria-controls="activity-list" `);
  return `<section class="activity-section" id="activity"><div class="section-heading"><h2>Last 30 changes</h2><div class="activity-actions">${countMarkup}${toggle}</div></div><div id="activity-list"${collapsed?' hidden':''}>${listMarkup}</div></section>`;
}

export function renderDemoBanner(): string {
  const reset=renderIconButton({
    label:'Reset demo',
    action:'demo-reset',
    icon:'reset',
    variant:'secondary demo-banner__reset',
  });
  return `<section class="demo-banner" aria-label="Demo"><div><p class="eyebrow">Demo</p><p>Explore four simulated blinds. Changes stay in this session.</p></div>${reset}</section>`;
}

export function renderIconLinkButton({label, action, icon, disabled = false}: IconButtonOptions): string {
  return renderButton({
    label,
    accessibleLabel:label,
    title:label,
    action,
    variant:'icon-link-button',
    disabled,
    icon,
    iconOnly:true,
  });
}

export function renderBackButton():string {
  return `<div class="back-navigation">${renderIconButton({label:'Back to blinds',action:'home',icon:'back',variant:'icon-link-button back-button'})}</div>`;
}

export function renderDestructiveButton({label, action, disabled = false, icon = 'trash'}: {label: string; action: string; disabled?: boolean; icon?: string}): string {
  return renderButton({label, action, variant:'danger', disabled, icon});
}

export function renderMotorSpeedControl({deviceId, busy = false, calibrationActive = false}: {deviceId: string; busy?: boolean; calibrationActive?: boolean}): string {
  const id = escapeHtml(deviceId);
  const disabled = busy || calibrationActive;
  return `<section class="panel device-settings"><h2>Motor speed</h2><p>Set the motor speed from 0% to 100%. SOMA controllers need firmware 2.2.0 or later. The current setting is not read by Somaris yet.</p><form data-motor-speed="${id}"><label class="field">Speed (%)<input name="motor_speed_percent" type="number" min="0" max="100" step="1" required placeholder="0–100"${disabled ? ' disabled' : ''}></label><button class="button"${disabled ? ' disabled' : ''}>Set speed</button></form></section>`;
}

export function renderCalibrationControls({deviceId, active = false, busy = false}: {deviceId: string; active?: boolean; busy?: boolean}): string {
  const id = String(deviceId ?? '');
  if (!active) {
    return renderButton({
      label:'Start endpoint calibration',
      action:`command:calibration_start:${id}`,
      variant:'secondary',
      disabled:busy,
    });
  }
  const controls = [
    ['Nudge open', 'calibration_nudge_open'],
    ['Set open limit', 'calibration_set_open_limit'],
    ['Nudge closed', 'calibration_nudge_closed'],
    ['Set closed limit', 'calibration_set_closed_limit'],
    ['Stop movement', 'stop'],
    ['End calibration', 'calibration_stop'],
  ] as const;
  return `<div class="calibration-actions">${controls.map(([label, action]) => renderButton({
    label,
    action:`command:${action}:${id}`,
    variant:action === 'calibration_stop' ? 'danger' : 'secondary',
    disabled:busy,
  })).join('')}</div>`;
}

export function renderConfirmationPanel({
  title,
  message,
  confirmLabel = 'Confirm',
  confirmAction,
  cancelAction,
  disabled = false,
}: {title: string; message: string; confirmLabel?: string; confirmAction: string; cancelAction: string; disabled?: boolean}): string {
  return `<section class="confirmation-panel panel" role="group" aria-label="${escapeHtml(title)}"><div class="confirmation-copy"><h2>${escapeHtml(title)}</h2><p>${escapeHtml(message)}</p></div><div class="confirmation-actions">${renderButton({label:'Cancel', action:cancelAction, variant:'secondary', disabled})}${renderDestructiveButton({label:confirmLabel, action:confirmAction, disabled})}</div></section>`;
}

export function renderAddBlindButton({variant = ''}: {variant?: string} = {}): string {
  return renderButton({
    label:'Add a blind', accessibleLabel:'Add a blind',
    action:'add',
    variant:['topbar-action', 'topbar-add', variant].filter(Boolean).join(' '),
    icon:'add',
  });
}

export function renderRoomGroup({room, actionsMarkup, managementMarkup}: {
  room:string;
  actionsMarkup?:string;
  managementMarkup?:string;
}): string {
  return renderRoomGroupWithActions({room,actionsMarkup,managementMarkup});
}

export function renderRoomGroupWithActions({room, actionsMarkup = '', managementMarkup = ''}: {
  room:string;
  actionsMarkup?:string;
  managementMarkup?:string;
}): string {
  return `<section class="room-group" data-room-group="${escapeHtml(room)}" aria-label="${escapeHtml(room)} room"><div class="room-group__heading"><h3 class="room-heading">${escapeHtml(room)}</h3>${actionsMarkup ? `<div class="room-group__actions">${actionsMarkup}</div>` : ''}</div>${managementMarkup}</section>`;
}

export function renderRoomCollection({groupsMarkup,cardsMarkup}:{groupsMarkup:string;cardsMarkup:string}):string {
  return `<div class="room-collection"><div class="room-collection__groups">${groupsMarkup}</div><div class="grid room-collection__grid">${cardsMarkup}</div></div>`;
}

export function roomChoiceValue(room: string): string {
  return `existing:${encodeURIComponent(room)}`;
}

export function roomFromChoice(choice: string, customName = ''): string | null {
  if (choice === 'custom') return customName.trim();
  if (choice === 'unassigned' || !choice) return null;
  if (!choice.startsWith('existing:')) return null;
  try {
    return decodeURIComponent(choice.slice('existing:'.length));
  } catch {
    return null;
  }
}

export function renderRoomPicker({id, rooms, selectedRoom = null}: RoomPickerOptions): string {
  const optionMarkup = rooms.map(room => {
    const value = roomChoiceValue(room);
    const selected = selectedRoom?.toLowerCase() === room.toLowerCase() ? ' selected' : '';
    return `<option value="${escapeHtml(value)}"${selected}>${escapeHtml(room)}</option>`;
  }).join('');
  const selectedValue = selectedRoom
    ? roomChoiceValue(selectedRoom)
    : 'unassigned';
  const customId = `${id}-custom`;
  return `<div class="room-picker"><label class="field" for="${escapeHtml(id)}">Room<select id="${escapeHtml(id)}" name="room_choice" data-room-choice aria-label="Room" aria-describedby="${escapeHtml(id)}-error"><option value="unassigned"${selectedValue === 'unassigned' ? ' selected' : ''}>Unassigned</option>${optionMarkup}<option value="custom">Create a room…</option></select></label><div class="room-picker__custom-field" data-room-custom-field hidden><label class="field" for="${escapeHtml(customId)}">New room name<input id="${escapeHtml(customId)}" name="room_custom" maxlength="40" autocomplete="off" placeholder="e.g. Living Room" data-room-custom-input aria-invalid="false" aria-describedby="${escapeHtml(customId)}-error"></label><span id="${escapeHtml(customId)}-error" class="field-error" data-room-error hidden role="alert"></span></div><span id="${escapeHtml(id)}-error" class="field-error" data-room-choice-error hidden role="alert"></span></div>`;
}

export function renderPositionPreset({deviceId, blindLabel, position, selectedPosition, disabled = false}: {
  deviceId:string;
  blindLabel:string;
  position:0|25|50|75|100;
  selectedPosition:number;
  disabled?:boolean;
}): string {
  const label=`Move ${blindLabel} to ${position}% open`;
  const encodedId=encodeURIComponent(deviceId);
  const markup=renderButton({
    label:`${position}%`,
    accessibleLabel:label,
    title:label,
    action:`position:${encodedId}:${position}`,
    variant:'position-preset',
    disabled,
  });
  return markup
    .replace('<button ',`<button aria-pressed="${position===selectedPosition}" `)
    .replace(' data-action=',` data-position-preset="${escapeHtml(encodedId)}:${position}" data-action=`);
}

export function renderPositionControl({deviceId, blindLabel, positionClosedPercent, disabled = false}: {
  deviceId:string;
  blindLabel:string;
  positionClosedPercent:number;
  disabled?:boolean;
}): string {
  const id=escapeHtml(deviceId);
  const positionOpenPercent=100-positionClosedPercent;
  const presets=([0,25,50,75,100] as const)
    .map(position=>renderPositionPreset({deviceId,blindLabel,position,selectedPosition:positionOpenPercent,disabled}))
    .join('');
  return `<div class="position-control" role="group" aria-label="Set position for ${escapeHtml(blindLabel)}"><div class="position-control__label"><label for="position-${id}">Set position</label><span><output data-position-output="${id}">${positionOpenPercent}%</output> open</span></div><div class="position-range"><span class="position-end position-end--closed">Closed</span><input id="position-${id}" type="range" min="0" max="100" step="1" value="${positionOpenPercent}" data-position-input="${id}" aria-label="Target position for ${escapeHtml(blindLabel)} in percent open"${disabled ? ' disabled' : ''}><span class="position-end position-end--open">Open</span></div><div class="position-presets" role="group" aria-label="Position presets for ${escapeHtml(blindLabel)}">${presets}</div></div>`;
}

export function renderTopbarActions({
  blindCount = 0,
  panelMode = false,
  demoMode = false,
  displayPreference = 'auto',
  serviceLabel = 'Connecting',
  serviceTone = '',
  themePreference = 'auto',
  serviceShortLabel,
  route = 'home',
}: TopbarActionsOptions = {}): string {
  const count = Number.isInteger(blindCount) && blindCount >= 0 ? blindCount : 0;
  const countLabel = `${count} ${count === 1 ? 'blind' : 'blinds'}`;
  const shortLabel=serviceShortLabel||(serviceTone==='good'?'Ready':serviceTone==='bad'?'Offline':'Check');
  const status = `<span id="service-status" class="service-status"><span class="status-dot${serviceTone ? ` ${escapeHtml(serviceTone)}` : ''}" aria-hidden="true"></span><span class="service-status__full">${escapeHtml(serviceLabel)}</span><span class="service-status__short">${escapeHtml(shortLabel)}</span></span>`;
  const panelCount = panelMode ? `<span class="topbar-device-count">${escapeHtml(countLabel)}</span>` : '';
  const layoutControl=renderLayoutControl({displayPreference});
  const themeControl=renderThemeControl({themePreference});
  const refreshButton = renderButton({label:'Refresh',accessibleLabel:'Refresh',action:'refresh',icon:'refresh',variant:'topbar-action topbar-icon-button topbar-action--compact-label'});
  const serviceButton = !panelMode ? renderIconButton({label:'Settings',action:'settings',icon:'settings',variant:'topbar-action topbar-icon-button'}) : '';
  const demoReset = panelMode && demoMode ? renderIconButton({label:'Reset demo',action:'demo-reset',icon:'reset',variant:'topbar-action topbar-icon-button'}) : '';
  return `${status}${panelCount}${serviceButton}${demoReset}${renderAddBlindButton()}${layoutControl}${themeControl}${refreshButton}`;
}

export function renderMobileTabs(route = 'home'): string {
  const active = route.startsWith('details/') || route === 'add' ? 'home' : route === 'service' ? 'settings' : route;
  const tabs = [
    ['home','Blinds','home'],['activity','Activity','activity'],['settings','Settings','settings'],
  ] as const;
  return `<nav class="mobile-tabs" aria-label="Main navigation">${tabs.map(([id,label,icon])=>`<a class="mobile-tab" href="#${id}" aria-label="${label}"${active===id?' aria-current="page"':''}>${iconMarkup(icon)}<span class="mobile-tab__label">${escapeHtml(label)}</span></a>`).join('')}</nav>`;
}

export function renderEmptyState({title, description, icon = null, actionMarkup = ''}: {
  title: string;
  description: string;
  icon?: string | null;
  actionMarkup?: string;
}): string {
  const illustration=icon ? `<div class="empty-icon empty-illustration">${iconMarkup(icon)}</div>` : '';
  return `<div class="panel empty">${illustration}<h3>${escapeHtml(title)}</h3><p>${escapeHtml(description)}</p>${actionMarkup}</div>`;
}

export function renderAddDeviceEmptyState({demoMode, adapterAvailable}: {demoMode: boolean; adapterAvailable: boolean}): string {
  if (demoMode) {
    return renderEmptyState({
      title:'Demo devices are ready',
      description:'This demo uses four simulated blinds. Adding Bluetooth devices is unavailable here.',
      icon:'home',
    });
  }
  const description=adapterAvailable
    ? 'Keep your blind nearby and refresh in a moment.'
    : 'The Bluetooth adapter is unavailable. Check the service status and try again.';
  return renderEmptyState({
    title:'No new blinds found',
    description,
    icon:'home',
    actionMarkup:renderIconButton({label:'Refresh devices',action:'refresh',icon:'refresh',variant:'secondary'}),
  });
}

export function renderHomeContent({deviceListMarkup, demoHintMarkup = '', supportingMarkup = ''}: {
  deviceListMarkup: string;
  demoHintMarkup?: string;
  supportingMarkup?: string;
}): string {
  return `${demoHintMarkup}${deviceListMarkup}${supportingMarkup}`;
}

export function renderDisclosure({label, contentMarkup, variant = ''}: {label: string; contentMarkup: string; variant?: string}): string {
  const classes=['disclosure',variant].filter(Boolean).map(escapeHtml).join(' ');
  const name=escapeHtml(label);
  return `<details class="${classes}"><summary class="disclosure__trigger" aria-label="${name}" title="${name}" data-tooltip="${name}">${iconMarkup('chevron')}</summary><div class="disclosure__content">${contentMarkup}</div></details>`;
}

export function renderBlindActions({deviceId, blindLabel, panelMode = false, disabled = false, busy = false, movementDisabled = false, supportsPosition = false, positionClosedPercent = 100, currentClosedPercent, currentPositionFresh = false}: BlindActionsOptions): string {
  const redundantAction=currentPositionFresh&&currentClosedPercent===0?'open':currentPositionFresh&&currentClosedPercent===100?'close':null;
  const primaryAction=busy||movementDisabled?'stop':currentPositionFresh&&currentClosedPercent===0?'close':'open';
  const actions = [
    ['open', 'Open'],
    ['stop', 'Stop'],
    ['close', 'Close'],
  ] as const;
  const disabledPosition=disabled||movementDisabled||busy;
  const positionControls=supportsPosition && !panelMode
    ? renderPositionControl({deviceId,blindLabel,positionClosedPercent,disabled:disabledPosition})
    : '';
  return `<div class="controls" role="group" aria-label="${escapeHtml(blindLabel)} controls">${actions.map(([action, label]) => {
    const accessibleLabel = `${label} ${blindLabel}`;
    const variant=[action===primaryAction?'button--primary':'secondary',panelMode?'panel-target':''].filter(Boolean).join(' ');
    return renderButton({
      label,
      accessibleLabel,
      action:`command:${action}:${deviceId}`,
      variant,
      disabled:disabled || action===redundantAction || (movementDisabled && action !== 'stop') || (busy && action !== 'stop'),
      icon:action,
      iconOnly:panelMode,
      title:panelMode ? accessibleLabel : '',
    });
  }).join('')}${positionControls}</div>`;
}

export function renderBatchFeedback(batch: BatchFeedbackView | null | undefined): string {
  return renderBatchFeedbackMarkup(batch,escapeHtml,renderButton);
}

export function renderBatchControls({blindCount = 0, eligibleCount = 0, disabled = false, busy = false, panelMode = false, batch = null}: BatchControlsOptions = {}): string {
  if (blindCount < 1 && !batch) return '';
  const unavailable = disabled || busy || eligibleCount < 1;
  const description = eligibleCount === 1
    ? 'Open or close the available blind.'
    : eligibleCount > 1
      ? `Open or close all ${eligibleCount} available blinds in sequence.`
      : 'No enrolled blinds are currently ready to move.';
  const panelVariant = panelMode ? ' all-blinds-controls--panel' : '';
  const controls = blindCount > 0
    ? `<section class="all-blinds-controls${panelVariant}" aria-label="All blinds controls"${busy ? ' aria-busy="true"' : ''}><div class="all-blinds-copy"><h2>All blinds</h2><p>${escapeHtml(description)}</p></div><div class="all-blinds-actions">${renderButton({label:'Open all', accessibleLabel:'Open all blinds', action:'batch:open', icon:'open', iconOnly:panelMode, title:panelMode ? 'Open all blinds' : '', variant:panelMode ? 'panel-target' : '', disabled:unavailable})}${renderButton({label:'Close all', accessibleLabel:'Close all blinds', action:'batch:close', icon:'close', iconOnly:panelMode, title:panelMode ? 'Close all blinds' : '', variant:panelMode ? 'secondary panel-target' : 'secondary', disabled:unavailable})}</div>${renderBatchFeedback(batch)}</section>`
    : '';
  return `${controls}${blindCount < 1 ? renderBatchFeedback(batch) : ''}`;
}

export function renderBlindCard({
  id,
  label,
  room,
  lastSeen,
  availabilityLabel,
  availabilityTone = '',
  availabilityStatus = 'unavailable',
  positionPercent,
  movementStatus,
  blindLines = 0,
  controlsMarkup,
  messageMarkup = '',
  footerMarkup = '',
  panelMode = false,
  supportsPosition = false,
  showPositionCue = false,
  showRoom = false,
}: BlindCardOptions): string {
  const slats = '<i></i>'.repeat(Math.max(0, Math.min(11, blindLines)));
  const fill=`<span class="blind-art__fill" style="--blind-fill:${Math.max(0,Math.min(100,positionPercent??0))}%">${slats}</span>`;
  const cardVariant = panelMode ? ' card--panel' : '';
  const displayVariant = panelMode ? ' blind-display--panel' : '';
  const header = renderDeviceCardHeader({id, label, availabilityLabel, availabilityTone, availabilityStatus, room, showRoom, lastSeen, panelMode});
  const status = renderBlindStatus({positionPercent, movementStatus, panelMode});
  const percentOpen=positionPercent===null||positionPercent===undefined?null:100-positionPercent;
  const graphic = supportsPosition && !panelMode
    ? `<div class="blind-art-wrap"><div class="blind-art blind-art--interactive" data-position-graphic="${escapeHtml(id)}" role="slider" tabindex="0" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${escapeHtml(percentOpen ?? 0)}" aria-valuetext="${escapeHtml(percentOpen===null?'Position unavailable':`${percentOpen}% open`)}" aria-label="Blind position for ${escapeHtml(label)}, percent open" data-movement="${escapeHtml(movementStatus)}">${fill}</div>${showPositionCue?'<span class="position-cue">Drag the blind to set position</span>':''}</div>`
    : `<div class="blind-art" data-movement="${escapeHtml(movementStatus)}" aria-hidden="true">${fill}</div>`;
  return `<article class="card${cardVariant}" data-device-card="${escapeHtml(id)}" data-room="${escapeHtml(room||'')}">${header}<div class="blind-display${displayVariant}">${status}${graphic}</div>${controlsMarkup}<div class="card-messages" data-card-messages="${escapeHtml(id)}">${messageMarkup}</div><div class="card-foot">${footerMarkup}</div></article>`;
}
