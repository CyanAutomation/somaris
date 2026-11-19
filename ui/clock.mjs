export function formatClockTime(date) {
    return new Intl.DateTimeFormat('en-GB', {
        hour: '2-digit',
        minute: '2-digit',
        hourCycle: 'h23',
    }).format(date);
}
export function renderClock(date = new Date()) {
    const value = formatClockTime(date);
    return `<time class="header-clock" data-clock datetime="${date.toISOString()}" aria-label="Local time, ${value}" title="Local time">${value}</time>`;
}
export function updateClock(clock, date) {
    const value = formatClockTime(date);
    clock.textContent = value;
    clock.setAttribute('datetime', date.toISOString());
    clock.setAttribute('aria-label', `Local time, ${value}`);
}
export function mountClock(slot, scheduler = {}) {
    const initialDate = scheduler.now?.() ?? new Date();
    slot.innerHTML = renderClock(initialDate);
    const clock = slot.querySelector('[data-clock]');
    return clock ? startClock(clock, scheduler) : () => { };
}
export function startClock(clock, scheduler = {}) {
    const now = scheduler.now ?? (() => new Date());
    const schedule = scheduler.setTimeout ?? ((callback, delay) => globalThis.setTimeout(callback, delay));
    const cancel = scheduler.clearTimeout ?? (handle => globalThis.clearTimeout(handle));
    let active = true;
    let timeout = null;
    const scheduleNextMinute = (date) => {
        const elapsedInMinute = date.getSeconds() * 1000 + date.getMilliseconds();
        timeout = schedule(tick, 60_000 - elapsedInMinute);
    };
    const tick = () => {
        timeout = null;
        if (!active)
            return;
        const date = now();
        updateClock(clock, date);
        scheduleNextMinute(date);
    };
    const initialDate = now();
    updateClock(clock, initialDate);
    scheduleNextMinute(initialDate);
    return () => {
        active = false;
        if (timeout !== null)
            cancel(timeout);
        timeout = null;
    };
}
