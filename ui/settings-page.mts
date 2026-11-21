import {
  movementStatusFromPositions,
  renderBackButton,
  renderBlindStatus,
  renderInfoGrid,
  renderServiceHealthSummary,
  renderThemePreferenceControl,
} from './components.mjs';
import {escapeHtml} from './escape.mjs';
import type {AdminAuthStatus, Diagnostics, SettingsSnapshot, SetupChecks, Simulator} from './types.mjs';

export function renderSettingsPage({
  themePreference,
  displayPreference,
  demoMode,
  diagnostics,
  simulators,
  managedCount,
  serviceError,
  serverSettings,
  serverSettingsError,
  adminAuth,
  adminAuthError,
  setupChecks,
}: {
  themePreference: string;
  displayPreference: string;
  demoMode: boolean;
  diagnostics: Diagnostics | null;
  simulators: Simulator[] | null;
  managedCount: number;
  serviceError: string | null;
  serverSettings: SettingsSnapshot | null;
  serverSettingsError: string | null;
  adminAuth: AdminAuthStatus | null;
  adminAuthError: string | null;
  setupChecks: SetupChecks | null;
}): string {
  const scanner = diagnostics?.scanner;
  const fields = demoMode
    ? [{label:'Simulated blinds',value:simulators?.length ?? managedCount}]
    : [
      {label:'Bluetooth adapter',value:scanner?.adapter_available ? 'Available' : 'Unavailable'},
      {label:'Scanner',value:scanner?.status || 'Unavailable'},
      {label:'Blind signals found',value:scanner?.soma_advertisements_seen ?? '—'},
      {label:'Last scan issue',value:scanner?.last_error_code || 'None reported'},
      {label:'Telemetry history',value:diagnostics?.history_enabled ? 'Enabled' : 'Not enabled'},
      {label:'Simulators',value:simulators?.length ?? 'Unavailable'},
    ];
  const notUsed = demoMode ? [
    {label:'Bluetooth adapter',value:'Not used here'},
    {label:'Scanner',value:'Not used here'},
    {label:'Blind signals found',value:'Not collected'},
    {label:'Scanner errors',value:'Not applicable'},
    {label:'Telemetry history',value:diagnostics?.history_enabled ? 'Enabled' : 'Not used here'},
  ] : [];
  const serviceSummary = serviceError
    ? 'The service connection needs attention'
    : demoMode
      ? diagnostics?.simulator_ready ? 'Everything is working' : 'The simulator is unavailable'
      : scanner?.adapter_available ? 'Everything is working' : 'Bluetooth scanning is unavailable';
  const notUsedMarkup = demoMode ? renderInfoGrid(notUsed) : '';
  const simulatorMarkup = demoMode ? '' : renderSimulatorControls(simulators);
  const settingsMarkup = serverSettingsError
    ? `<section class="panel settings-service-config"><h2>Service configuration</h2><p class="message warn" role="alert">${escapeHtml(serverSettingsError)}</p></section>`
    : serverSettings
      ? renderServiceSettings(serverSettings, demoMode, adminAuth, adminAuthError)
      : '<section class="panel settings-service-config"><h2>Service configuration</h2><p class="subtle" role="status">Loading service settings…</p></section>';
  return `${renderBackButton()}<h1 class="page-title">Settings</h1>${renderThemePreferenceControl(themePreference,displayPreference)}${settingsMarkup}${renderSetupChecks(setupChecks)}<section class="settings-service"><h2>Service status</h2>${renderServiceHealthSummary({summary:serviceSummary,fields,notUsedMarkup})}${simulatorMarkup}</section>`;
}

function renderServiceSettings(
  settings: SettingsSnapshot,
  demoMode: boolean,
  adminAuth: AdminAuthStatus | null,
  adminAuthError: string | null,
): string {
  const rows = [
    ['simulator_enabled','Simulated blinds',settings.values.simulator_enabled,'boolean'],
    ['allowed_origins','Browser access from other sites',settings.values.allowed_origins,'origins'],
    ['log_level','Log level',settings.values.log_level,'log_level'],
    ['history_api_enabled','Telemetry history API',settings.values.history_api_enabled,'boolean'],
    ['history_retention','History retention',settings.values.history_retention,'number'],
    ['queue_max_outstanding','Maximum outstanding commands',settings.values.queue_max_outstanding,'number'],
    ['queue_max_queued_per_device','Maximum queued commands per blind',settings.values.queue_max_queued_per_device,'number'],
    ['adapter_concurrency','Adapter concurrency',settings.values.adapter_concurrency,'number'],
    ['adapter_scan_window_ms','Bluetooth scan window',settings.values.adapter_scan_window_ms,'number'],
    ['queue_wait_timeout_seconds','Queue wait timeout',settings.values.queue_wait_timeout_seconds,'number'],
  ] as const;
  const authenticated = adminAuth?.authenticated === true;
  const renderRow = ([key,label,setting,kind]: typeof rows[number]): string => {
    const value = Array.isArray(setting.value)
      ? setting.value.join(', ') || 'None'
      : typeof setting.value === 'boolean'
        ? key === 'history_api_enabled' ? setting.value ? 'Enabled' : 'Disabled' : setting.value ? 'On' : 'Off'
        : String(setting.value);
    const control = demoMode || setting.locked || !authenticated
      ? `<p class="setting-value">${escapeHtml(value)}</p>`
      : renderSettingControl(key,label,kind,setting.value);
    const source = setting.source === 'env'
      ? `Set by ${environmentVariableFor(key)}; remove it from your environment to manage it here.`
      : setting.source === 'file' ? 'Saved in /data/settings.json.' : 'Built-in default.';
    const restart = setting.requires_restart ? ' Restart required to apply changes.' : '';
    return `<div class="settings-row" data-setting="${key}"><div><strong>${label}</strong>${control}<p class="subtle">${escapeHtml(source + restart)}</p><p class="subtle setting-constraints">Allowed: ${escapeHtml(setting.constraints.join(', '))}</p></div></div>`;
  };
  const authPanel = renderAdminAuth(demoMode,adminAuth,adminAuthError,authenticated);
  const saveForm = `<form data-service-settings><div class="settings-rows">${rows.slice(0,5).map(renderRow).join('')}</div><h3>Advanced</h3><div class="settings-rows settings-advanced">${rows.slice(5).map(renderRow).join('')}</div><div class="settings-actions"><button class="button" type="submit"${!authenticated||demoMode?' disabled':''}>Save service settings</button>${authenticated&&!demoMode?'<button class="button secondary" type="button" data-action="settings-reset">Reset service settings</button><button class="button secondary" type="button" data-action="service-restart">Restart service</button>':''}</div></form>`;
  const content = demoMode
    ? `<p class="notice">Service settings are read-only in demo mode.</p><div class="settings-rows">${rows.slice(0,5).map(renderRow).join('')}</div><h3>Advanced</h3><div class="settings-rows settings-advanced">${rows.slice(5).map(renderRow).join('')}</div>`
    : `${authPanel}${saveForm}`;
  const pending = settings.pending_restart.length
    ? `<p class="message warn">Restart required for: ${escapeHtml(settings.pending_restart.join(', '))}.</p>` : '';
  return `<section class="panel settings-service-config"><h2>Service</h2>${content}${pending}</section>`;
}

function renderSettingControl(key: string,label: string,kind: string,value: unknown):string {
  const id=`setting-${key}`;
  if(kind==='boolean'){
    const selected=value===true?'true':'false';
    return `<label class="field" for="${id}"><span>${label}</span><select id="${id}" name="${key}"><option value="true"${selected==='true'?' selected':''}>On</option><option value="false"${selected==='false'?' selected':''}>Off</option></select></label>`;
  }
  if(kind==='log_level'){
    const current=String(value);
    return `<label class="field" for="${id}"><span>${label}</span><select id="${id}" name="${key}">${['error','warn','info','debug'].map(level=>`<option value="${level}"${current===level?' selected':''}>${level}</option>`).join('')}</select></label>`;
  }
  if(kind==='origins'){
    const origins=Array.isArray(value)?value.join(', '):'';
    return `<label class="field" for="${id}"><span>${label}</span><input id="${id}" name="${key}" type="text" value="${escapeHtml(origins)}" placeholder="https://dashboard.example" autocomplete="off"></label>`;
  }
  const ranges:Record<string,[number,number,number]>={history_retention:[100,500000,1],queue_max_outstanding:[1,1000,1],queue_max_queued_per_device:[1,1000,1],adapter_concurrency:[1,64,1],adapter_scan_window_ms:[100,10000,1],queue_wait_timeout_seconds:[1,86400,1]};
  const [min,max,step]=ranges[key]||[1,1000,1];
  return `<label class="field" for="${id}"><span>${label}</span><input id="${id}" name="${key}" type="number" min="${min}" max="${max}" step="${step}" value="${Number(value)}"></label>`;
}

function environmentVariableFor(key:string):string {
  const names:Record<string,string>={
    simulator_enabled:'SOMARIS_SIMULATOR',allowed_origins:'SOMARIS_ALLOWED_ORIGINS',log_level:'RUST_LOG',
    history_api_enabled:'SOMARIS_HISTORY_API_ENABLED or SOMARIS_HISTORY_PATH',history_retention:'SOMARIS_HISTORY_RETENTION',
    queue_max_outstanding:'SOMARIS_COMMAND_MAX_OUTSTANDING',queue_max_queued_per_device:'SOMARIS_COMMAND_MAX_QUEUED_PER_DEVICE',
    adapter_concurrency:'SOMARIS_ADAPTER_CONCURRENCY',adapter_scan_window_ms:'SOMARIS_ADAPTER_SCAN_WINDOW_MS',
    queue_wait_timeout_seconds:'SOMARIS_COMMAND_QUEUE_WAIT_TIMEOUT_SECONDS',
  };
  return names[key]||'an environment variable';
}

function renderAdminAuth(demoMode:boolean,auth:AdminAuthStatus|null,error:string|null,authenticated:boolean):string {
  if(demoMode)return '';
  if(error)return `<p class="message warn" role="alert">${escapeHtml(error)}</p>`;
  if(!auth)return '<p class="subtle" role="status">Checking administrator access…</p>';
  if(auth.setup_required)return `<section class="settings-auth"><h3>Set administrator passphrase</h3><p>Read the one-time setup code from <code>docker compose logs somaris</code>, then choose a passphrase with at least 12 characters.</p><form data-admin-setup><label class="field">Setup code<input name="code" required autocomplete="one-time-code"></label><label class="field">New passphrase<input name="passphrase" type="password" minlength="12" maxlength="256" required autocomplete="new-password"></label><button class="button" type="submit">Set passphrase</button></form></section>`;
  if(!authenticated)return `<section class="settings-auth"><h3>Administrator sign in</h3><form data-admin-login><label class="field">Passphrase<input name="passphrase" type="password" maxlength="256" required autocomplete="current-password"></label><button class="button" type="submit">Sign in</button></form></section>`;
  return '<div class="settings-auth"><p class="subtle">Signed in as the Somaris administrator.</p><button class="button secondary" type="button" data-action="admin-logout">Sign out</button></div>';
}

function renderSetupChecks(checks:SetupChecks|null):string {
  if(!checks)return '<section class="panel settings-setup-checks"><h2>Setup checks</h2><p class="subtle" role="status">Checking Bluetooth and storage…</p></section>';
  const rows=checks.checks.map(check=>`<article class="setup-check setup-check--${escapeHtml(check.status)}"><div><strong>${escapeHtml(check.label)}</strong><p>${escapeHtml(check.message)}</p>${check.fix?`<pre><code>${escapeHtml(check.fix)}</code></pre>`:''}</div></article>`).join('');
  return `<section class="panel settings-setup-checks"><h2>Setup checks</h2><p class="subtle">Checks explain common Compose, Bluetooth, and storage issues.</p><div class="setup-checks">${rows}</div></section>`;
}

function renderSimulatorControls(simulators: Simulator[] | null): string {
  if (!simulators?.length) return '';
  type SimulatorOption = readonly [string,string];
  const options = (values: readonly SimulatorOption[], current: string): string => values
    .map(([value,label]) => `<option value="${value}" ${value===current?'selected':''}>${label}</option>`)
    .join('');
  const scenarios = [['normal','Normal'],['pre_write_failure','Connection failure'],['outcome_unknown','Unknown outcome'],['gatt_write_error','Write error'],['telemetry_paused','Telemetry paused']] as const;
  const profiles = [['standard','Standard'],['hallway_observed','Hallway observed'],['hallway_slow_link','Hallway slow link']] as const;
  const batteries = [['unsupported','Unsupported'],['unavailable','Unavailable'],['observed_sample','Observed sample']] as const;
  const instances = simulators.map(simulator => {
    const movementStatus = movementStatusFromPositions(simulator.current_closed_percent,simulator.target_closed_percent);
    const position = renderBlindStatus({positionPercent:simulator.current_closed_percent,movementStatus,compact:true});
    return `<div class="simulator-controls"><h3>${escapeHtml(simulator.advertised_name)}</h3><div class="simulator-position">${position}<span class="subtle">Simulator ID ${escapeHtml(simulator.id)}</span></div><form data-simulator="${escapeHtml(simulator.id)}"><label class="field">Scenario<select name="scenario">${options(scenarios,simulator.scenario ?? '')}</select></label><label class="field">Wireless profile<select name="wireless_profile">${options(profiles,simulator.wireless_profile ?? '')}</select></label><label class="field">Battery state<select name="battery_state">${options(batteries,simulator.battery_state ?? '')}</select></label><button class="button">Apply settings</button></form><button class="button secondary" style="margin-top:12px" data-action="sim-reset:${escapeHtml(simulator.id)}">Reset to closed</button></div>`;
  }).join('');
  return `<section class="panel" style="margin-top:28px"><p class="eyebrow">Development</p><h2>Simulator controls</h2><p class="subtle">Settings and resets apply only to the selected simulator.</p>${instances}</section>`;
}
