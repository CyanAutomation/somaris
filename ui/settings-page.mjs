import { movementStatusFromPositions, renderBackButton, renderBlindStatus, renderButton, renderInfoGrid, renderServiceHealthSummary, renderThemePreferenceControl, } from './components.mjs';
import { escapeHtml } from './escape.mjs';
export function renderSettingsPage({ themePreference, demoMode, diagnostics, simulators, managedCount, serviceError, }) {
    const scanner = diagnostics?.scanner;
    const fields = demoMode
        ? [{ label: 'Simulated blinds', value: simulators?.length ?? managedCount }]
        : [
            { label: 'Bluetooth adapter', value: scanner?.adapter_available ? 'Available' : 'Unavailable' },
            { label: 'Scanner', value: scanner?.status || 'Unavailable' },
            { label: 'Blind signals found', value: scanner?.soma_advertisements_seen ?? '—' },
            { label: 'Last scan issue', value: scanner?.last_error_code || 'None reported' },
            { label: 'Telemetry history', value: diagnostics?.history_enabled ? 'Enabled' : 'Not enabled' },
            { label: 'Simulators', value: simulators?.length ?? 'Unavailable' },
        ];
    const notUsed = demoMode ? [
        { label: 'Bluetooth adapter', value: 'Not used here' },
        { label: 'Scanner', value: 'Not used here' },
        { label: 'Blind signals found', value: 'Not collected' },
        { label: 'Scanner errors', value: 'Not applicable' },
        { label: 'Telemetry history', value: diagnostics?.history_enabled ? 'Enabled' : 'Not used here' },
    ] : [];
    const serviceSummary = serviceError
        ? 'The service connection needs attention'
        : demoMode
            ? diagnostics?.simulator_ready ? 'Everything is working' : 'The simulator is unavailable'
            : scanner?.adapter_available ? 'Everything is working' : 'Bluetooth scanning is unavailable';
    const notUsedMarkup = demoMode ? renderInfoGrid(notUsed) : '';
    const simulatorMarkup = demoMode ? '' : renderSimulatorControls(simulators);
    return `${renderBackButton()}<h1 class="page-title">Settings</h1>${renderThemePreferenceControl(themePreference)}<section class="settings-service"><h2>Service status</h2>${renderServiceHealthSummary({ summary: serviceSummary, fields, notUsedMarkup })}${simulatorMarkup}</section>`;
}
function renderSimulatorControls(simulators) {
    if (!simulators?.length)
        return '';
    const options = (values, current) => values
        .map(([value, label]) => `<option value="${value}" ${value === current ? 'selected' : ''}>${label}</option>`)
        .join('');
    const scenarios = [['normal', 'Normal'], ['pre_write_failure', 'Connection failure'], ['outcome_unknown', 'Unknown outcome'], ['gatt_write_error', 'Write error'], ['telemetry_paused', 'Telemetry paused']];
    const profiles = [['standard', 'Standard'], ['hallway_observed', 'Hallway observed'], ['hallway_slow_link', 'Hallway slow link']];
    const batteries = [['unsupported', 'Unsupported'], ['unavailable', 'Unavailable'], ['observed_sample', 'Observed sample']];
    const instances = simulators.map(simulator => {
        const movementStatus = movementStatusFromPositions(simulator.current_closed_percent, simulator.target_closed_percent);
        const position = renderBlindStatus({ positionPercent: simulator.current_closed_percent, movementStatus, compact: true });
        return `<div class="simulator-controls"><h3>${escapeHtml(simulator.advertised_name)}</h3><div class="simulator-position">${position}<span class="subtle">Simulator ID ${escapeHtml(simulator.id)}</span></div><form data-simulator="${escapeHtml(simulator.id)}"><label class="field">Scenario<select name="scenario">${options(scenarios, simulator.scenario ?? '')}</select></label><label class="field">Wireless profile<select name="wireless_profile">${options(profiles, simulator.wireless_profile ?? '')}</select></label><label class="field">Battery state<select name="battery_state">${options(batteries, simulator.battery_state ?? '')}</select></label><button class="button">Apply settings</button></form><button class="button secondary" style="margin-top:12px" data-action="sim-reset:${escapeHtml(simulator.id)}">Reset to closed</button></div>`;
    }).join('');
    return `<section class="panel" style="margin-top:28px"><p class="eyebrow">Development</p><h2>Simulator controls</h2><p class="subtle">Settings and resets apply only to the selected simulator.</p>${instances}</section>`;
}
