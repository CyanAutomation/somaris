import {test} from 'node:test';
import assert from 'node:assert/strict';
import {SnackbarQueue} from '../snackbar.mjs';

function fakeScheduler() {
  let now = 0;
  let nextId = 0;
  const timers = new Map();
  return {
    now:() => now,
    setTimeout(callback, delay) {
      const id = ++nextId;
      timers.set(id, {callback, at:now + delay});
      return id;
    },
    clearTimeout(id) { timers.delete(id); },
    advance(milliseconds) {
      const end = now + milliseconds;
      while (true) {
        const due = [...timers.entries()].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        now = due[1].at;
        timers.delete(due[0]);
        due[1].callback();
      }
      now = end;
    },
    timerCount:() => timers.size,
  };
}

test('snackbar queue deduplicates active messages and caps its visible stack', () => {
  const scheduler = fakeScheduler();
  const queue = new SnackbarQueue({scheduler, maxVisible:2});
  const first = queue.push({message:'Disconnected',tone:'error',key:'service'});
  assert.equal(queue.push({message:'Disconnected again',tone:'error',key:'service'}), first);
  assert.equal(queue.items.length, 1);
  assert.equal(queue.items[0].message, 'Disconnected');

  queue.push({message:'Saved',tone:'success'});
  queue.push({message:'Another result',tone:'info'});
  assert.deepEqual(queue.items.map(item=>item.message), ['Disconnected','Saved','Another result']);
  assert.deepEqual(queue.visibleItems.map(item=>item.message), ['Disconnected','Saved']);
  assert.equal(scheduler.timerCount(), 2);
  queue.dismiss(first);
  assert.deepEqual(queue.visibleItems.map(item=>item.message), ['Saved','Another result']);
  assert.equal(scheduler.timerCount(), 2);
  queue.destroy();
  assert.equal(scheduler.timerCount(), 0);
});

test('snackbar timeout pauses on interaction and resumes with its remaining time', () => {
  const scheduler = fakeScheduler();
  const queue = new SnackbarQueue({scheduler});
  const itemId = queue.push({message:'Saved',tone:'success',duration:5000});

  scheduler.advance(2000);
  queue.pause(itemId,'pointer');
  queue.pause(itemId,'focus');
  scheduler.advance(10000);
  assert.equal(queue.items.length, 1);

  queue.resume(itemId,'pointer');
  scheduler.advance(1000);
  assert.equal(queue.items.length, 1);
  queue.resume(itemId,'focus');
  scheduler.advance(2999);
  assert.equal(queue.items.length, 1);
  scheduler.advance(1);
  assert.equal(queue.items.length, 0);
  queue.destroy();
});

test('permanent snackbars remain until dismissed and enqueue changes are observable', () => {
  const scheduler = fakeScheduler();
  const changes = [];
  const queue = new SnackbarQueue({scheduler,onChange:items=>changes.push(items.length)});
  const item = queue.push({message:'Check position',tone:'warning',duration:0,key:'uncertain'});
  scheduler.advance(60000);
  assert.equal(queue.items.length, 1);
  assert.equal(queue.dismissByKey('uncertain'), true);
  assert.equal(queue.items.length, 0);
  assert.deepEqual(changes, [1,0]);
  queue.destroy();
});
