import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseHTML} from 'linkedom';
import {formatClockTime, mountClock, renderClock, startClock} from '../clock.mjs';

test('clock formats local time with a 24-hour clock and leading zeroes', () => {
  assert.equal(formatClockTime(new Date(2026, 10, 19, 0, 4)), '00:04');
  assert.equal(formatClockTime(new Date(2026, 10, 19, 9, 7)), '09:07');
  assert.equal(formatClockTime(new Date(2026, 10, 19, 17, 42)), '17:42');
});

test('clock renders a semantic time element with a machine-readable timestamp', () => {
  const date = new Date(2026, 10, 19, 17, 42);
  const document = parseHTML(`<!doctype html><html><body>${renderClock(date)}</body></html>`).document;
  const clock = document.querySelector('time[data-clock]');

  assert.ok(clock);
  assert.equal(clock.textContent, '17:42');
  assert.equal(clock.getAttribute('datetime'), date.toISOString());
  assert.equal(clock.getAttribute('aria-label'), 'Local time, 17:42');
});

test('clock mounts into a reusable slot and returns a cleanup function', () => {
  const date = new Date(2026, 10, 19, 6, 5);
  const document = parseHTML('<!doctype html><html><body></body></html>').document;
  const slot = document.createElement('span');
  let cancelled = false;

  const stop = mountClock(slot, {
    now: () => date,
    setTimeout: () => 1,
    clearTimeout: () => {cancelled = true;},
  });

  assert.equal(slot.querySelector('time[data-clock]')?.textContent, '06:05');
  stop();
  assert.equal(cancelled, true);
});

test('clock updates on the minute boundary and cancels its next update when stopped', () => {
  let now = new Date(2026, 10, 19, 17, 8, 42, 250);
  const timers: Array<{callback: () => void; delay: number; cancelled: boolean}> = [];
  const document = parseHTML(`<!doctype html><html><body>${renderClock(now)}</body></html>`).document;
  const clock = document.querySelector('time[data-clock]');
  assert.ok(clock);

  const stop = startClock(clock as unknown as HTMLTimeElement, {
    now: () => now,
    setTimeout: (callback, delay) => {
      const timer = {callback, delay, cancelled:false};
      timers.push(timer);
      return timer;
    },
    clearTimeout: handle => {
      (handle as typeof timers[number]).cancelled = true;
    },
  });

  assert.equal(timers[0]?.delay, 17_750);
  now = new Date(2026, 10, 19, 17, 9, 0);
  timers[0]?.callback();
  assert.equal(clock.textContent, '17:09');
  assert.equal(clock.getAttribute('aria-label'), 'Local time, 17:09');
  assert.equal(timers[1]?.delay, 60_000);

  stop();
  assert.equal(timers[1]?.cancelled, true);
});
