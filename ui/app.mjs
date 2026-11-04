import {
  escapeHtml,
  renderBlindActions,
  renderBlindCard,
  renderBlindStatus,
  renderAddBlindButton,
  renderButton,
  renderDeviceList,
  renderHomeSupportingSections,
  renderInfoGrid,
  movementStatusFromPositions,
} from './components.mjs';

export {escapeHtml};
const displayModes = new Set(['auto', 'panel', 'mobile', 'desktop']);
export function resolveDisplayMode(preference, width, height) {
  if (displayModes.has(preference) && preference !== 'auto') return preference;
  const viewportWidth = Number.isFinite(width) ? Math.max(0, width) : 0;
  const viewportHeight = Number.isFinite(height) ? Math.max(0, height) : 0;
  const aspectRatio = viewportHeight ? viewportWidth / viewportHeight : 0;
  if (viewportWidth > 0 && viewportHeight > 0 && Math.max(viewportWidth, viewportHeight) <= 1100 && aspectRatio >= 0.82 && aspectRatio <= 1.22) return 'panel';
  return viewportWidth <= 600 ? 'mobile' : 'desktop';
}
export function displayPreferenceFrom(search, saved) {
  const requested = new URLSearchParams(search || '').get('view');
  if (displayModes.has(requested)) return requested;
  return displayModes.has(saved) ? saved : 'auto';
}
function storedDisplayPreference() {
  let saved;
  try {
    saved = globalThis.localStorage?.getItem('somaris-display-mode');
  } catch { /* Storage can be unavailable in private browsing contexts. */ }
  return displayPreferenceFrom(globalThis.location?.search, saved);
}
export function commandMessage(command) {
  switch (command.status) {
    case 'queued': return ['Queued. Waiting for the blind…', ''];
    case 'connecting': return ['Sending command to the blind…', ''];
    case 'write_acknowledged': return ['Command acknowledged. Movement is shown when reported by the blind.', ''];
    case 'outcome_unknown': case 'write_failed': return ['The outcome is uncertain. Check the reported position before trying again.', 'warn'];
    case 'connection_failed': return [command.retryable ? 'Could not connect. No command was sent; you can try again.' : 'Could not connect to the blind.', 'bad'];
    case 'expired': return ['The command expired before it could be sent.', 'bad'];
    case 'cancelled': return ['The command was cancelled.', 'warn'];
    default: return ['Checking command status…', ''];
  }
}
export function activityMessage(activity) {
  const subject = activity.subject || 'Somaris';
  const action = activity.action ? `${activity.action[0].toUpperCase()}${activity.action.slice(1)}` : '';
  const status = activity.status?.replaceAll('_', ' ');
  switch (activity.kind) {
    case 'enrollment_added': return `Added ${subject}`;
    case 'enrollment_removed': return `Removed ${subject}`;
    case 'command': return `${action} command${status ? ` · ${status}` : ''} · ${subject}`;
    case 'batch': return `${action} batch${status ? ` · ${status}` : ''} · ${subject}`;
    case 'simulator_settings_changed': return `Updated simulator settings · ${subject}${activity.detail ? ` · ${activity.detail}` : ''}`;
    case 'simulator_reset': return `Reset simulator · ${subject}${activity.detail ? ` · ${activity.detail}` : ''}`;
    default: return `Changed ${subject}`;
  }
}
export function observationFor(managed, observations) {
  const e = managed.enrollment;
  const matches = observations.filter(o => o.logical_id === e.logical_id && o.advertised_name === e.advertised_name && !!o.is_simulated === !!e.is_simulated);
  return matches.length === 1 ? matches[0] : null;
}
export function batteryDisplay(battery) {
  if (!Number.isInteger(battery?.percent_estimate) || battery.percent_estimate < 0 || battery.percent_estimate > 100) return '<span class="subtle">Battery unavailable</span>';
  const cached = battery.status !== 'observed' ? ' · cached' : '';
  const voltage = Number.isInteger(battery.voltage_mv) && battery.voltage_mv > 0 ? `Battery voltage: ${battery.voltage_mv} mV. ` : '';
  return `<div class="battery-reading subtle">Battery ~${battery.percent_estimate}%${cached}<details class="battery-info"><summary aria-label="Battery information">i</summary><span class="battery-tooltip">${voltage}Percentage is an estimate based on voltage.</span></details></div>`;
}
const terminal = new Set(['write_acknowledged','connection_failed','write_failed','outcome_unknown','expired','cancelled']);
const app = typeof document === 'undefined' ? null : document.querySelector('#app');
const state = {managed:[], observations:[], diagnostics:null, activity:[], commands:{}, serviceError:null, enrollmentError:null, actionError:null, busy:new Set(), tokens:{}, simulators:null};
const esc = escapeHtml;
const relative = date => { if (!date) return 'Never seen'; const seconds = Math.max(0, Math.floor((Date.now()-new Date(date).getTime())/1000)); if (!Number.isFinite(seconds)) return 'Time unavailable'; if(seconds < 60) return 'Seen just now'; if(seconds < 3600) return `Seen ${Math.floor(seconds/60)} min ago`; if(seconds < 86400) return `Seen ${Math.floor(seconds/3600)} hr ago`; return `Seen ${Math.floor(seconds/86400)} days ago`; };
async function api(path, options) { const response = await fetch(path, {cache:'no-store', ...options}); const body = await response.json().catch(()=>({})); if(!response.ok) throw new Error(body.error?.message || body.message || `Request failed (${response.status})`); return body.data; }
function button(label, action, extra='', disabled=false, icon=null) {
 return renderButton({label, action, variant:extra, disabled, icon});
}
function notice(message, dismissAction) { return message ? `<div class="notice" role="alert">${esc(message)}${dismissAction?` <button class="text-button" data-action="${dismissAction}">Dismiss</button>`:''}</div>` : ''; }
function availability(value) { return ({ready:['Ready',''],stale:['Stale','warn'],not_seen:['Not seen','warn'],ambiguous:['Ambiguous','bad']})[value] || ['Unavailable','warn']; }
function card(device) {
 const e=device.enrollment, o=observationFor(device,state.observations), [label,tone]=availability(device.availability), p=device.position?.current_closed_percent;
 const panelMode=document.documentElement.dataset.displayMode==='panel';
 const command=state.commands[e.id]; const [message,msgTone]=command ? commandMessage(command) : ['', ''];
 const lines=Number.isInteger(p) ? Math.max(1, Math.round(p/100*11)) : 0;
 const disabled=!!state.serviceError||device.availability!=='ready';
 const controlsMarkup=renderBlindActions({
  deviceId:e.id, blindLabel:e.label,
  panelMode,
  disabled, busy:state.busy.has(e.id),
 });
 const messageMarkup=message?`<p class="message ${msgTone}" role="status">${esc(message)}</p>`:'';
 const availabilityMessage=device.availability!=='ready'?'<p class="message warn">Controls are unavailable until this blind is uniquely observed again.</p>':'';
 const footerMarkup=`${batteryDisplay(o?.battery)}<button type="button" class="text-button" data-action="details:${esc(e.id)}">View details &nbsp; ↗</button>`;
 return renderBlindCard({
  id:e.id, label:e.label, isSimulated:e.is_simulated, lastSeen:relative(device.last_seen_at),
  availabilityLabel:label, availabilityTone:tone,
  positionPercent:p, movementStatus:o?.movement_status, blindLines:lines,
  controlsMarkup, messageMarkup:`${messageMarkup}${availabilityMessage}`, footerMarkup, panelMode,
 });
}
function activityPanel() {
 const entries = state.activity || [];
 const list = entries.length
  ? `<ol class="activity-list">${entries.map(entry=>`<li class="activity-item"><span class="activity-copy">${esc(activityMessage(entry))}${entry.detail&&entry.kind!=='simulator_settings_changed'&&entry.kind!=='simulator_reset'?`<span class="subtle">${esc(entry.detail)}</span>`:''}</span><time datetime="${esc(entry.completed_at||entry.occurred_at)}">${esc(new Date(entry.completed_at||entry.occurred_at).toLocaleString())}</time></li>`).join('')}</ol>`
  : '<div class="panel activity-empty"><p>No changes recorded yet.</p></div>';
 return `<section class="activity-section"><div class="section-heading"><h2>Recent activity</h2><span class="count">${entries.length} of 30 changes</span></div>${list}</section>`;
}
function home() {
 const panelMode = document.documentElement.dataset.displayMode === 'panel';
 const hero = panelMode ? '' : `<section class="hero"><div><p class="eyebrow">A little more light</p><h1>Your space, your way.</h1><p>A quieter way to keep your blinds just where you like them.</p></div>${renderAddBlindButton({variant:'hero-add'})}</section>`;
 const add = panelMode ? renderAddBlindButton({variant:'secondary panel-add'}) : '';
 const empty = panelMode
  ? `<div class="panel empty"><h3>No blinds yet</h3><p>Add a nearby blind to see its status and controls here.</p></div>`
  : `<div class="panel empty"><div class="empty-icon">☼</div><h3>Let’s bring your blinds in.</h3><p>When a blind is nearby and discovered, add it here to see its status and control it.</p>${renderAddBlindButton()}</div>`;
 const listMarkup=renderDeviceList({
  count:state.managed.length, panelMode, addButton:add,
  cardsMarkup:state.managed.map(card).join(''), emptyMarkup:empty,
 });
 const serviceMarkup=`<div class="section-heading" style="margin-top:35px"><h2>Somaris service</h2><button type="button" class="text-button" data-action="service">View service status &nbsp; ↗</button></div><p class="subtle">${esc(scannerSummary())}</p>`;
 const supportingSections=renderHomeSupportingSections({
  panelMode,
  activityMarkup:panelMode?'':activityPanel(),
  serviceMarkup:panelMode?'':serviceMarkup,
 });
 return `${hero}${notice(state.serviceError)}${notice(state.actionError,'dismiss-action')}${listMarkup}${supportingSections}`;
}
function scannerSummary(){ const s=state.diagnostics?.scanner; if(!s) return 'Service status unavailable'; return s.adapter_available ? (s.scan_active?'Bluetooth adapter available · scanning':'Bluetooth adapter available') : 'Bluetooth adapter unavailable · retrying'; }
function back(){ return '<button class="text-button back" data-action="home">← &nbsp; Back to your blinds</button>'; }
function addPage(){ const available=state.observations.filter(o=>!state.managed.some(m=>m.enrollment.logical_id===o.logical_id && !!m.enrollment.is_simulated===!!o.is_simulated)); return `${back()}<h1 class="page-title">Add a blind</h1><p class="page-intro">Choose a recently discovered blind to make it available for control.</p>${notice(state.serviceError)}${notice(state.enrollmentError,'dismiss-enrollment')}<div class="section-heading"><h2>Nearby devices</h2><span class="count">${available.length} found</span></div>${available.length?`<div class="stack">${available.map(o=>`<form class="panel device-row" data-enroll="${esc(o.id)}"><div><h3>${esc(o.advertised_name||o.device_name||'Unnamed blind')}</h3><p class="subtle">${o.is_simulated?'Simulator · ':''}${esc(relative(o.last_seen_at))} · ${esc(o.freshness)}</p><label class="field">Display name<input name="label" maxlength="64" required value="${esc(o.advertised_name||o.device_name||'Blind')}"></label></div><button class="button" ${o.freshness!=='fresh'?'disabled':''}>Add blind</button></form>`).join('')}</div>`:`<div class="panel empty"><div class="empty-icon">⌁</div><h3>No new blinds found</h3><p>${state.diagnostics?.scanner?.adapter_available?'Keep your blind nearby and refresh in a moment.': 'The Bluetooth adapter is unavailable. Check the service status and try again.'}</p>${button('Refresh devices','refresh','secondary')}</div>`}`; }
function detailPage(id){
 const d=state.managed.find(x=>x.enrollment.id===id);
 if(!d)return `${back()}<div class="notice">This blind is no longer enrolled.</div>`;
 const o=observationFor(d,state.observations);
 const fields=[
  {label:'Availability',value:availability(d.availability)[0]},
  {label:'Position',contentMarkup:renderBlindStatus({positionPercent:d.position?.current_closed_percent,movementStatus:o?.movement_status,compact:true})},
  {label:'Last observation',value:d.last_seen_at?new Date(d.last_seen_at).toLocaleString():'Never'},
  {label:'Reported target',value:d.position?`${d.position.target_closed_percent}% closed`:'Unavailable'},
  {label:'Signal',value:o?.rssi_dbm!=null?`${o.rssi_dbm} dBm`:'Unavailable'},
  {label:'Battery voltage',value:o?.battery?.voltage_mv!=null?`${o.battery.voltage_mv} mV${o.battery.status!=='observed'?' (cached)':''}`:'Unavailable'},
  {label:'Source',value:d.enrollment.is_simulated?'Simulator':'Bluetooth'},
 ];
 return `${back()}<h1 class="page-title">${esc(d.enrollment.label)}</h1><p class="page-intro">The latest observations reported by your blind.</p>${renderInfoGrid(fields)}<p class="message">Position is observed telemetry and may not reflect the current physical position.</p>`;
}
function simulatorControls(){
 if(!state.simulators?.length)return '';
 const options=(values,current)=>values.map(([value,label])=>`<option value="${value}" ${value===current?'selected':''}>${label}</option>`).join('');
 const scenarios=[['normal','Normal'],['pre_write_failure','Connection failure'],['outcome_unknown','Unknown outcome'],['gatt_write_error','Write error'],['telemetry_paused','Telemetry paused']];
 const profiles=[['standard','Standard'],['hallway_observed','Hallway observed'],['hallway_slow_link','Hallway slow link']];
 const batteries=[['unsupported','Unsupported'],['unavailable','Unavailable'],['observed_sample','Observed sample']];
 const instances=state.simulators.map(simulator=>{
  const movementStatus=movementStatusFromPositions(simulator.current_closed_percent,simulator.target_closed_percent);
  const position=renderBlindStatus({positionPercent:simulator.current_closed_percent,movementStatus,compact:true});
  return `<div class="simulator-controls"><h3>${esc(simulator.advertised_name)}</h3><div class="simulator-position">${position}<span class="subtle">Simulator ID ${esc(simulator.id)}</span></div><form data-simulator="${esc(simulator.id)}"><label class="field">Scenario<select name="scenario">${options(scenarios,simulator.scenario)}</select></label><label class="field">Wireless profile<select name="wireless_profile">${options(profiles,simulator.wireless_profile)}</select></label><label class="field">Battery state<select name="battery_state">${options(batteries,simulator.battery_state)}</select></label><button class="button">Apply settings</button></form><button class="button secondary" style="margin-top:12px" data-action="sim-reset:${esc(simulator.id)}">Reset to closed</button></div>`;
 }).join('');
 return `<section class="panel" style="margin-top:28px"><p class="eyebrow">Development</p><h2>Simulator controls</h2><p class="subtle">Settings and resets apply only to the selected simulator.</p>${instances}</section>`;
}
function servicePage(){ const s=state.diagnostics?.scanner; const fields=[['Service',state.serviceError?'Connection problem':'Connected'],['Bluetooth adapter',s?.adapter_available?'Available':'Unavailable'],['Scanner',s?.status||'Unavailable'],['Advertisements seen',s?.soma_advertisements_seen??'—'],['Last scanner error',s?.last_error_code||'None reported'],['Telemetry history',state.diagnostics?.history_enabled?'Enabled':'Not enabled'],['Simulators',state.simulators?.length??'Unavailable or disabled']].map(([label,value])=>({label,value})); return `${back()}<h1 class="page-title">Service status</h1><p class="page-intro">A closer look at the Somaris service and Bluetooth scanner.</p>${notice(state.serviceError)}${notice(state.actionError,'dismiss-action')}${renderInfoGrid(fields)}${simulatorControls()}`; }
function page(){return location.hash.slice(1)||'home'}
function render(){ if(!app)return; const focused=app.contains(document.activeElement) ? document.activeElement.dataset.action : null; const p=page(); app.innerHTML=p==='add'?addPage():p==='service'?servicePage():p.startsWith('details/')?detailPage(decodeURIComponent(p.slice(8))):home(); const indicator=document.querySelector('#service-status'); indicator.innerHTML=`<span class="status-dot ${state.serviceError?'bad':state.diagnostics?.scanner?.adapter_available?'good':''}"></span> ${state.serviceError?'Disconnected':state.diagnostics?.scanner?.adapter_available?'Connected':'Service degraded'}`; if(focused) [...app.querySelectorAll('[data-action]')].find(el=>el.dataset.action===focused)?.focus({preventScroll:true}); }
export async function refreshService(target, request) {
  try {
    const [managed, observations, diagnostics, simulators, activity] = await Promise.all([
      request('/api/control/v1/devices'), request('/api/v1/devices'),
      request('/api/v1/diagnostics'), request('/api/dev/simulators').catch(() => null),
      request('/api/control/v1/audit-log'),
    ]);
    Object.assign(target, {managed, observations, diagnostics, simulators, activity, serviceError:null});
  } catch (error) {
    target.serviceError = `Could not refresh Somaris: ${error.message}`;
  }
}

export function dismissEnrollmentError(target) { target.enrollmentError = null; }

export async function enrollBlind(target, request, refreshData, observedDeviceId, label) {
  dismissEnrollmentError(target);
  try {
    await request('/api/control/v1/devices', {
      method:'POST', headers:{'Content-Type':'application/json'},
      body:JSON.stringify({observed_device_id:observedDeviceId, label}),
    });
    await refreshData();
    return true;
  } catch (error) {
    await refreshData();
    target.enrollmentError = `Could not add blind: ${error.message.replace(/[.!?]+$/, "")}. Refresh and check the list before retrying.`;
    return false;
  }
}

async function refresh(quiet=false){ await refreshService(state, api); if(!quiet || !app?.querySelector('input:focus, select:focus')) render(); }
async function sendCommand(id,action){ if(state.busy.has(id) && action!=='stop') return; const token=Symbol(); state.busy.add(id); state.actionError=null; state.tokens[id]=token; state.commands[id]={status:'queued'}; render(); try { const key=globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random().toString(36).slice(2)}`; const command=await api(`/api/control/v1/devices/${encodeURIComponent(id)}/commands`,{method:'POST',headers:{'Content-Type':'application/json','Idempotency-Key':key},body:JSON.stringify({action})}); if(state.tokens[id]!==token)return; state.commands[id]=command; render(); pollCommand(id,command.id,token); } catch(e) { if(state.tokens[id]!==token)return; state.commands[id]={status:'outcome_unknown'}; state.actionError=`Command request failed: ${e.message}. Its outcome may be unknown; check telemetry before trying again.`; state.busy.delete(id); render(); } }
async function pollCommand(id,commandId,token){ try { const command=await api(`/api/control/v1/commands/${encodeURIComponent(commandId)}`); if(state.tokens[id]!==token)return; state.commands[id]=command; render(); if(!terminal.has(command.status)){ setTimeout(()=>pollCommand(id,commandId,token),1500); return; } state.busy.delete(id); await refresh(true); } catch(e){ if(state.tokens[id]!==token)return; state.commands[id]={status:'outcome_unknown'}; state.busy.delete(id); state.actionError=`Could not check command status: ${e.message}. Check telemetry before trying again.`; render(); } }
async function simulatorReset(id){state.actionError=null; try{await api(`/api/dev/simulators/${encodeURIComponent(id)}/reset`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({position_closed_percent:100})}); await refresh();}catch(error){state.actionError=`Simulator reset failed: ${error.message}`; render();}}
if(app){
 let displayPreference = storedDisplayPreference();
 const displaySelector = document.querySelector('#display-mode');
 const applyDisplayMode = () => {
  const previous = document.documentElement.dataset.displayMode;
  document.documentElement.dataset.displayMode = resolveDisplayMode(displayPreference, window.innerWidth, window.innerHeight);
  return previous !== document.documentElement.dataset.displayMode;
 };
 displaySelector.value = displayPreference;
 applyDisplayMode();
 displaySelector.addEventListener('change', () => {
  displayPreference = displaySelector.value;
  try { globalThis.localStorage?.setItem('somaris-display-mode', displayPreference); } catch { /* The current selection still applies until the page reloads. */ }
  if (applyDisplayMode()) render();
 });
 window.addEventListener('resize', () => { if (applyDisplayMode()) render(); });
 window.addEventListener('hashchange',render); document.querySelector('#refresh').addEventListener('click',()=>refresh()); app.addEventListener('click', e=>{const target=e.target.closest('[data-action]'); if(!target)return; const [action,arg,id]=target.dataset.action.split(':'); if(action==='command')sendCommand(id,arg); else if(action==='details')location.hash=`details/${encodeURIComponent(arg)}`; else if(action==='refresh')refresh(); else if(action==='sim-reset')simulatorReset(arg); else if(action==='dismiss-enrollment'){dismissEnrollmentError(state);render();} else if(action==='dismiss-action'){state.actionError=null;render();} else location.hash=action;}); app.addEventListener('submit',async e=>{ if(e.target.matches('[data-simulator]')){e.preventDefault(); const values=Object.fromEntries(new FormData(e.target)); const id=e.target.dataset.simulator; state.actionError=null; try{await api(`/api/dev/simulators/${encodeURIComponent(id)}/scenario`,{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify(values)}); await refresh();}catch(error){state.actionError=`Simulator settings could not be saved: ${error.message}`; render();} return;} const form=e.target.closest('[data-enroll]'); if(!form)return; e.preventDefault(); const b=form.querySelector('button'); b.disabled=true; const id=form.dataset.enroll, label=new FormData(form).get('label').trim(); const added=await enrollBlind(state,api,()=>refreshService(state,api),id,label); if(added) location.hash='home'; render();}); refresh(); setInterval(()=>refresh(true),5000);
}
