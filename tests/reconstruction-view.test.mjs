/** Display navigation is independent from device work and validated result loading. */
import assert from 'node:assert/strict';import{readFileSync}from'node:fs';import test from'node:test';
const read=path=>readFileSync(new URL('../'+path,import.meta.url),'utf8');const app=read('src/App.tsx');const recon=read('src/ReconstructionConsole.tsx');
test('equipment canvas remains mounted behind every central media layer',()=>{
  const scene=app.slice(app.indexOf('function LiveScene('),app.indexOf('function XrayPanel('));
  assert.match(scene,/<LiveSceneCanvas/);assert.match(scene,/visibility: centerView === "equipment" \? "visible" : "hidden"/);
  assert.match(scene,/<WorkstationMedia/);assert.match(scene,/<ControlDock/);assert.doesNotMatch(scene,/key=\{centerView/);
});
test('equipment context listener retires before renderer disposal',()=>{
  const source=read('src/scene/LiveSceneCanvas.tsx');assert.match(source,/return \(\) => canvas.removeEventListener\("webglcontextlost", handleContextLost\)/);
});
test('reconstruction starts a requested method and keeps equipment visible',()=>{
  const source=app.slice(app.indexOf('const startReconstruction'),app.indexOf('const onPhoto'));
  assert.match(source,/navigate\("equipment"\)/);assert.match(source,/type:\s*"start_reconstruction"/);
});
test('late result loading validates bytes and cannot steal a later user view',()=>{
  const loader=app.slice(app.indexOf('if (!previewKey || adapterKind'),app.indexOf('const openReconstruction'));
  assert.ok(loader.indexOf('Invalid preview volume data')<loader.indexOf('setPreviewData'));
  assert.match(loader,/revision === presentation.current/);assert.match(loader,/if \(!active\) return/);
});
test('view buttons never start reconstruction and events are backend-owned',()=>{
  const open=app.slice(app.indexOf('const openReconstruction'),app.indexOf('const startReconstruction'));
  assert.doesNotMatch(open,/actions.run|dispatch/);assert.match(recon,/onClick=\{onShowEquipment\}/);assert.match(recon,/onClick=\{onOpenReconstruction\}/);
  assert.doesNotMatch(recon,/setEvents|eventSequence|recon-workspace__events/);
});
