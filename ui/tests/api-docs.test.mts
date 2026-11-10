import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseHTML} from 'linkedom';
import {loadOpenApiReference} from '../api-docs.mjs';

function apiReferenceDocument() {
  return parseHTML(`<!doctype html><html><head><title>API reference</title></head><body>
    <h1>API reference</h1><p id="status"></p><div id="operations"></div>
  </body></html>`).document;
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers:{'content-type':'application/json'},
  });
}

test('API reference renders supported operations and treats descriptions as text', async t => {
  // Contract: UI-API-DOC-01 in docs/test-contracts.md.
  const document = apiReferenceDocument();
  const documentValue = {
    info:{title:'Somaris API', version:'1'},
    paths:{
      '/devices':{
        get:{summary:'List devices'},
        post:{description:'Enroll a <img src=x onerror=alert(1)> device'},
        trace:{summary:'Unsupported method'},
        delete:'malformed operation',
      },
      '/ignored':'not a path item',
    },
  };
  t.mock.method(globalThis, 'fetch', async input => {
    assert.equal(input, '/openapi.json');
    return jsonResponse(documentValue);
  });

  await loadOpenApiReference(document as unknown as Document);

  assert.equal(document.title, 'Somaris API reference');
  assert.equal(document.querySelector('h1')?.textContent, 'Somaris API reference');
  assert.equal(document.querySelector('#status')?.textContent, '');
  const rows = [...document.querySelectorAll('#operations section')];
  assert.equal(rows.length, 2);
  assert.deepEqual(rows.map(row => [
    row.querySelector('span')?.textContent,
    row.querySelector('code')?.textContent,
    row.querySelector('p')?.textContent,
  ]), [
    ['GET', '/devices', 'List devices'],
    ['POST', '/devices', 'Enroll a <img src=x onerror=alert(1)> device'],
  ]);
  assert.equal(document.querySelector('#operations img'), null);
});

test('API reference reports a malformed document in its status region', async t => {
  // Contract: UI-API-DOC-01 in docs/test-contracts.md.
  const malformed = apiReferenceDocument();
  t.mock.method(globalThis, 'fetch', async () => jsonResponse({info:{}, paths:[]}));

  await loadOpenApiReference(malformed as unknown as Document);

  assert.equal(malformed.querySelector('#status')?.textContent,
    'Could not load the OpenAPI document: The OpenAPI document has an unexpected format.');
  assert.equal(malformed.querySelectorAll('#operations section').length, 0);
});

test('API reference reports an unavailable contract without building operation rows', async t => {
  // Contract: UI-API-DOC-01 in docs/test-contracts.md.
  const unavailable = apiReferenceDocument();
  t.mock.method(globalThis, 'fetch', async () => jsonResponse({}, 503));

  await loadOpenApiReference(unavailable as unknown as Document);

  assert.equal(unavailable.querySelector('#status')?.textContent,
    'Could not load the OpenAPI document: HTTP 503');
  assert.equal(unavailable.querySelectorAll('#operations section').length, 0);
});
