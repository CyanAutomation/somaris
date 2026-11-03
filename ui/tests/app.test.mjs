import {test} from 'node:test';
import assert from 'node:assert/strict';
import {commandMessage, escapeHtml, observationFor} from '../app.mjs';

test('command messages distinguish acknowledgement and uncertain outcomes',()=>{
 assert.match(commandMessage({status:'write_acknowledged'})[0],/acknowledged/i);
 assert.match(commandMessage({status:'outcome_unknown'})[0],/check.*position/i);
 assert.match(commandMessage({status:'write_failed'})[0],/uncertain/i);
 assert.match(commandMessage({status:'connection_failed',retryable:true})[0],/no command was sent/i);
});
test('observations are associated only with a unique matching identity and source',()=>{
 const d={enrollment:{logical_id:'x',advertised_name:'Blind',is_simulated:false}};
 const o={logical_id:'x',advertised_name:'Blind',is_simulated:false};
 assert.equal(observationFor(d,[o]),o);
 assert.equal(observationFor(d,[o,o]),null);
 assert.equal(observationFor(d,[{...o,is_simulated:true}]),null);
});
test('device-provided text is escaped',()=>assert.equal(escapeHtml('<img src="x">'), '&lt;img src=&quot;x&quot;&gt;'));

test('enrollment errors survive successful and failed service refreshes until retry or dismissal', async () => {
 const {enrollBlind, refreshService, dismissEnrollmentError} = await import('../app.mjs');
 const state = {enrollmentError:null, serviceError:null};
 let failRefresh = false;
 const request = async (path, options) => {
  if (options?.method === 'POST') throw new Error('Device is no longer fresh');
  if (failRefresh) throw new Error('Network unavailable');
  return path.endsWith('/diagnostics') ? {scanner:{adapter_available:true}} : [];
 };
 const refresh = () => refreshService(state, request);
 assert.equal(await enrollBlind(state, request, refresh, 'blind-1', 'Hallway'), false);
 assert.match(state.enrollmentError, /Device is no longer fresh/);
 assert.equal(state.serviceError, null);
 await refresh();
 assert.match(state.enrollmentError, /Device is no longer fresh/);
 failRefresh = true;
 await refresh();
 assert.match(state.serviceError, /Network unavailable/);
 assert.match(state.enrollmentError, /Device is no longer fresh/);
 dismissEnrollmentError(state);
 assert.equal(state.enrollmentError, null);
 assert.match(state.serviceError, /Network unavailable/);
 failRefresh = false;
 const successfulRequest = async (path, options) => options?.method === 'POST' ? {} : request(path, options);
 state.enrollmentError = 'previous failure';
 assert.equal(await enrollBlind(state, successfulRequest, refresh, 'blind-1', 'Hallway'), true);
 assert.equal(state.enrollmentError, null);
 assert.equal(state.serviceError, null);
});
