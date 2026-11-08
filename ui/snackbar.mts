import type {Notification, Scheduler, SnackbarItem, Tone} from './types.mjs';

const durations: Record<Tone, number> = {info:5500,success:5500,warning:8000,error:8000};
let nextSnackbarId = 0;

interface TimerState {
  handle: unknown | null;
  dueAt: number;
  remaining: number;
  paused: Set<string>;
}

interface SchedulerConfig {
  onChange?: (visibleItems: SnackbarItem[], allItems: SnackbarItem[]) => void;
  scheduler?: Scheduler;
  maxVisible?: number;
}

function defaultScheduler(): Scheduler {
  return {
    now:() => Date.now(),
    setTimeout:(callback, delay) => globalThis.setTimeout(callback, delay),
    clearTimeout:id => {
      if (typeof id === 'number') globalThis.clearTimeout(id);
      else if (id !== null) globalThis.clearTimeout(id as ReturnType<typeof globalThis.setTimeout>);
    },
  };
}

export class SnackbarQueue {
  #items: SnackbarItem[] = [];
  #timers = new Map<string, TimerState>();
  #scheduler: Scheduler;
  #onChange: NonNullable<SchedulerConfig['onChange']>;
  #maxVisible: number;

  constructor({onChange = () => {}, scheduler = defaultScheduler(), maxVisible = 3}: SchedulerConfig = {}) {
    this.#onChange = onChange;
    this.#scheduler = scheduler;
    this.#maxVisible = Math.max(1, Number.isInteger(maxVisible) ? maxVisible : 3);
  }

  get items(): SnackbarItem[] { return this.#items.map(item => ({...item})); }
  get visibleItems(): SnackbarItem[] { return this.#items.slice(0, this.#maxVisible).map(item => ({...item})); }

  push({message, tone = 'info', key = null, action = null, duration}: Notification = {message: ''}): string | null {
    if (typeof message !== 'string' || !message.trim()) return null;
    if (key) {
      const existing = this.#items.find(item => item.key === key);
      if (existing) return existing.id;
    }
    const safeTone: Tone = Object.hasOwn(durations, tone) ? tone : 'info';
    const timeout = typeof duration === 'number' && Number.isFinite(duration) ? Math.max(0, duration) : durations[safeTone];
    const item: SnackbarItem = {
      id:`snackbar-${++nextSnackbarId}`,
      key,
      message,
      tone:safeTone,
      action:action && typeof action.label === 'string' && typeof action.action === 'string' ? {...action} : null,
      duration:timeout,
    };
    this.#items.push(item);
    this.#syncTimers();
    this.#emit();
    return item.id;
  }

  pause(id: string, reason = 'interaction'): void {
    const timer = this.#timers.get(id);
    if (!timer || timer.paused.has(reason)) return;
    if (!timer.paused.size) {
      timer.remaining = Math.max(0, timer.dueAt - this.#scheduler.now());
      this.#scheduler.clearTimeout(timer.handle);
      timer.handle = null;
    }
    timer.paused.add(reason);
  }

  resume(id: string, reason = 'interaction'): void {
    const timer = this.#timers.get(id);
    if (!timer || !timer.paused.delete(reason) || timer.paused.size) return;
    this.#schedule(id, timer, timer.remaining);
  }

  dismiss(id: string): boolean {
    const index = this.#items.findIndex(item => item.id === id);
    if (index < 0) return false;
    this.#clearTimer(id);
    this.#items.splice(index, 1);
    this.#syncTimers();
    this.#emit();
    return true;
  }

  dismissByKey(key: string): boolean {
    const item = this.#items.find(candidate => candidate.key === key);
    return item ? this.dismiss(item.id) : false;
  }

  destroy() {
    for (const id of this.#timers.keys()) this.#clearTimer(id);
    this.#items = [];
  }

  #schedule(id: string, timer: TimerState, delay: number): void {
    timer.remaining = delay;
    timer.dueAt = this.#scheduler.now() + delay;
    timer.handle = this.#scheduler.setTimeout(() => this.dismiss(id), delay);
  }

  #clearTimer(id: string): void {
    const timer = this.#timers.get(id);
    if (!timer) return;
    if (timer.handle !== null) this.#scheduler.clearTimeout(timer.handle);
    this.#timers.delete(id);
  }

  #syncTimers() {
    const activeIds = new Set(this.#items.slice(0, this.#maxVisible).map(item => item.id));
    for (const id of this.#timers.keys()) {
      if (!activeIds.has(id)) this.#clearTimer(id);
    }
    for (const item of this.#items.slice(0, this.#maxVisible)) {
      if (item.duration <= 0 || this.#timers.has(item.id)) continue;
      const timer: TimerState = {handle:null,dueAt:0,remaining:item.duration,paused:new Set()};
      this.#timers.set(item.id, timer);
      this.#schedule(item.id, timer, item.duration);
    }
  }

  #emit(): void { this.#onChange(this.visibleItems, this.items); }
}
