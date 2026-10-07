/** Every declared action is wired to one shared route; menus never mutate drafts by opening folders. */
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
const root=new URL('../',import.meta.url);
const read=path=>readFileSync(new URL(path,root),'utf8');
const model=read('src/menuActions.ts');const app=read('src/App.tsx');
test('five functional groups contain unique actions with concrete handlers',()=>{
  const ids=[...model.matchAll(/id: "([a-zA-Z]+\.[a-zA-Z]+)"/g)].map(match=>match[1]);
  assert.equal(ids.length,new Set(ids).size);assert.equal(ids.length,30);
  for(const id of ids) assert.match(app,new RegExp(`case\\s+"${id.replace('.','\\.')}":`),id);
  for(const group of ['Task','Devices','View','Tools','Help']) assert.ok(model.includes(`label: "${group}"`));
  assert.doesNotMatch(model,/edit\.undo|edit\.redo|homeAllAxes/);
});
test('folder navigation never changes the common capture root',()=>{
  const branch=app.slice(app.indexOf('case "task.openFolder":'),app.indexOf('case "devices.camera":'));
  assert.match(branch,/revealDirectory/);assert.doesNotMatch(branch,/chooseImageDirectory|set_capture_root|update_scan_setup/);
  const panel=read('src/CameraPanel.tsx');assert.match(panel,/type: "set_capture_root", savePath: selectedRoot/);
});
test('Help updater and offline import remain platform actions before snapshot readiness',()=>{
  const handling=app.slice(app.indexOf('const handleMenuAction'),app.indexOf('const info ='));
  assert.match(handling,/case "help.checkUpdate": setUpdate\("check"\)/);assert.match(handling,/case "help.importUpdate": setUpdate\("import"\)/);
  assert.doesNotMatch(handling,/if \(!ws\) return/);
  const boot=app.slice(app.indexOf('if (!snapshot || !ws) return'),app.indexOf('const submitPreparation'));
  assert.match(boot,/<MenuBar/);assert.match(boot,/<UpdateDialog/);
  assert.match(app,/getApplicationVersion\(\)/);assert.doesNotMatch(app,/APP_VERSION\s*=|v0\.8\.2/);
});
test('restore, abandon, logs, reconstruction and independent HOME retain their routes',()=>{
  assert.match(app,/case "task.restore": case "task.abandon": setAuxiliary\("recovery"\)/);
  assert.match(read('src/TaskRecoveryDialog.tsx'),/type: "abandon_scan"/);
  assert.match(app,/case "devices.turntableHome": await actions.run\(\{ type:"turntable_home"/);
  assert.match(app,/case "view.reconstruction": setBottomTab\("reconstruction"\)/);
  assert.match(app,/case "view.logs": setBottomTab\("aggregate"\)/);
  assert.match(app,/case "tools.exportLog":/);
});
test('declared menu availability and shared callbacks drive actual chrome',()=>{
  assert.match(app,/availability=\{availability\}/);assert.match(app,/onAction=\{id => void handleMenuAction\(id\)\}/);
  assert.match(app,/snapshot\?\.deviceControls.taskLock.locked/);
  assert.doesNotMatch(read('src/CameraSettingsDialog.tsx'),/camera_live_view_stop|useState<CameraProperties>/);
});
