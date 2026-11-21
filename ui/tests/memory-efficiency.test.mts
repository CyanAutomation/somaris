import {test} from 'node:test';
import assert from 'node:assert/strict';
import {windowPage} from '../pagination.mjs';
import {createLazyModule} from '../lazy-module.mjs';
import {renderAddDevicePage} from '../add-page.mjs';
import {refreshService, shouldPauseRefresh} from '../app.mjs';
import type {ApiRequest, Observation} from '../types.mjs';

test('lazy module stays available synchronously after its first load',async()=>{
 let loads=0;
 const module={render:()=>'<main>ready</main>'};
 const lazyModule=createLazyModule(async()=>{
  loads++;
  return module;
 });

 assert.equal(lazyModule.loaded,undefined);
 const first=lazyModule.load();
 const second=lazyModule.load();
 assert.strictEqual(first,second);
 assert.equal(loads,1);
 assert.strictEqual(await first,module);
 assert.strictEqual(lazyModule.loaded,module);
 assert.equal(lazyModule.loaded?.render(),'<main>ready</main>');
 assert.strictEqual(await lazyModule.load(),module);
 assert.equal(loads,1);
});

test('windowPage returns only the requested prefix and a bounded next page',()=>{
 const items=Array.from({length:100},(_,index)=>index);
 const page=windowPage(items,24);
 assert.deepEqual(page.items,items.slice(0,24));
 assert.equal(page.total,100);
 assert.equal(page.hasMore,true);
 assert.equal(page.nextLimit,48);
});

test('windowPage handles empty and final pages without retaining extra items',()=>{
 assert.deepEqual(windowPage([],24),{items:[],total:0,hasMore:false,nextLimit:0});
 assert.deepEqual(windowPage([1,2,3],24),{items:[1,2,3],total:3,hasMore:false,nextLimit:3});
 assert.deepEqual(windowPage([1,2,3],2),{items:[1,2],total:3,hasMore:true,nextLimit:3});
});

test('nearby device page renders forms only for its visible window',()=>{
 const observations:Observation[]=Array.from({length:100},(_,index)=>({
  id:`device-${index}`,
  logical_id:`logical-${index}`,
  advertised_name:`Blind ${index}`,
  freshness:'fresh',
  last_seen_at:'2026-11-21T00:00:00.000Z',
 }));
 const page=renderAddDevicePage({
  observations,
  managed:[],
  rooms:[],
  demoMode:false,
  adapterAvailable:true,
  visibleCount:24,
  now:Date.parse('2026-11-21T00:00:00.000Z'),
 });
 assert.equal((page.match(/<form class="panel device-row"/g)||[]).length,24);
 assert.match(page,/100 found/);
 assert.match(page,/Showing 24 of 100/);
 assert.match(page,/data-action="more-nearby"/);
});

test('background refresh pauses while the document is hidden',()=>{
 assert.equal(shouldPauseRefresh({documentHidden:true}),true);
 assert.equal(shouldPauseRefresh({documentHidden:false}),false);
});

test('concurrent service refreshes share one request batch', {timeout:5000}, async()=>{
 let release!:()=>void;
 const gate=new Promise<void>(resolve=>{release=resolve;});
 let announceStarted!:()=>void;
 const requestsStarted=new Promise<void>(resolve=>{announceStarted=resolve;});
 const calls:string[]=[];
 const request:ApiRequest=async path=>{
  calls.push(path);
  if(calls.length===5)announceStarted();
  await gate;
  return path==='/api/v1/diagnostics'?{scanner:{adapter_available:true}}:[];
 };
 const target={};

 const first=refreshService(target,request);
 const second=refreshService(target,request);
 assert.strictEqual(first,second);
 await requestsStarted;
 assert.equal(calls.length,5);
 release();
 await Promise.all([first,second]);
 assert.equal(calls.length,5);
});
