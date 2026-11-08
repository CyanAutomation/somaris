import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseOpenApiDocument} from '../api-docs.mjs';

test('OpenAPI parser keeps supported operations and ignores malformed entries', () => {
  const document = parseOpenApiDocument({
    info:{title:'Somaris API', version:'1'},
    paths:{
      '/devices':{
        get:{summary:'List devices'},
        post:{description:'Enroll a device'},
        trace:{summary:'Unsupported method'},
        delete:'malformed operation',
      },
      '/ignored':'not a path item',
    },
  });

  assert.deepEqual(document, {
    info:{title:'Somaris API'},
    paths:{'/devices':{
      get:{summary:'List devices'},
      post:{description:'Enroll a device'},
    }},
  });
});

test('OpenAPI parser rejects a missing title or paths map', () => {
  assert.throws(() => parseOpenApiDocument({info:{}, paths:{}}), /unexpected format/);
  assert.throws(() => parseOpenApiDocument({info:{title:'Somaris API'}, paths:[]}), /unexpected format/);
});
