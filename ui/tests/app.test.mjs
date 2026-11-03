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
