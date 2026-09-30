/** Regression guards for independent display navigation and deferred result loading. */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
const source = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
const between = (start,end) => source.slice(source.indexOf(start),source.indexOf(end,source.indexOf(start)));

test('starting reconstruction keeps equipment visible and submits only the requested method', () => {
  const start=between('const startReconstruction =','const availability:');
  assert.match(start,/setCenterView\("equipment"\)/);
  assert.doesNotMatch(start,/setCenterView\("reconstruction"\)/);
  assert.match(start,/dispatch\(\{ type: "start_reconstruction", method \}\)/);
});

test('completed preview loads in background and auto-switches only after validation', () => {
  const loader=between('if (!previewKey || adapterKind', 'const openReconstruction');
  assert.doesNotMatch(loader,/centerView !== "reconstruction"/);
  assert.ok(loader.indexOf('Invalid preview volume data') < loader.indexOf('setPreviewData'));
  assert.ok(loader.indexOf('setPreviewData') < loader.indexOf('setCenterView("reconstruction")'));
  assert.match(loader,/previewData\?\.key === previewKey/);
  assert.match(loader,/if \(!active\) return/);
});

test('display navigation never dispatches a reconstruction and does not depend on live IPC', () => {
  const open=between('const openReconstruction =','const startReconstruction =');
  assert.match(open,/if \(activeVolume\)/);
  assert.doesNotMatch(open,/dispatch|transportError|running/);
  assert.match(source,/onClick=\{onShowEquipment\}>Equipment/);
  assert.match(source,/disabled=\{!completed\}.*onClick=\{onOpenReconstruction\}>Result/);
  assert.match(open,/setPreviewRetry/);
  const primary=between('const activatePrimary =', 'return <div className="recon-workspace">');
  assert.match(primary,/start\(\)/);
  assert.doesNotMatch(primary,/onShowEquipment|onOpenReconstruction/);
});
