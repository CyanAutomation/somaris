export function formatClockTime(date: Date): string {
  return new Intl.DateTimeFormat('en-GB', {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(date);
}

export function renderClock(date = new Date()): string {
  const value = formatClockTime(date);
  return `<time class="header-clock" data-clock datetime="${date.toISOString()}" aria-label="Local time, ${value}" title="Local time">${value}</time>`;
}

export function updateClock(clock: HTMLTimeElement, date: Date): void {
  const value = formatClockTime(date);
  clock.textContent = value;
  clock.setAttribute('datetime', date.toISOString());
  clock.setAttribute('aria-label', `Local time, ${value}`);
}

export interface ClockScheduler {
  now?: () => Date;
  setTimeout?: (callback: () => void, delay: number) => unknown;
  clearTimeout?: (handle: unknown) => void;
}

export function mountClock(slot: HTMLElement, scheduler: ClockScheduler = {}): () => void {
  const initialDate = scheduler.now?.() ?? new Date();
  slot.innerHTML = renderClock(initialDate);
  const clock = slot.querySelector<HTMLTimeElement>('[data-clock]');
  return clock ? startClock(clock, scheduler) : () => {};
}

export function startClock(clock: HTMLTimeElement, scheduler: ClockScheduler = {}): () => void {
  const now = scheduler.now ?? (() => new Date());
  const schedule = scheduler.setTimeout ?? ((callback, delay) => globalThis.setTimeout(callback, delay));
  const cancel = scheduler.clearTimeout ?? (handle => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>));
  let active = true;
  let timeout: unknown = null;

  const scheduleNextMinute = (date: Date): void => {
    const elapsedInMinute = date.getSeconds() * 1000 + date.getMilliseconds();
    timeout = schedule(tick, 60_000 - elapsedInMinute);
  };
  const tick = (): void => {
    timeout = null;
    if (!active) return;
    const date = now();
    updateClock(clock, date);
    scheduleNextMinute(date);
  };

  const initialDate = now();
  updateClock(clock, initialDate);
  scheduleNextMinute(initialDate);

  return () => {
    active = false;
    if (timeout !== null) cancel(timeout);
    timeout = null;
  };
}
