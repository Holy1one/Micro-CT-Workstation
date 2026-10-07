/** Domain parity exercises actual browser adapter source. No hardware or image files. */
import assert from "node:assert/strict";
import test from "node:test";
import { loadSourceModule } from "./load-source-module.mjs";
const {WorkstationAdapter}=await loadSourceModule("src/engine/workstationAdapter.ts");
const storage=new Map();globalThis.window={localStorage:{getItem:key=>storage.get(key)??null,setItem:(key,value)=>storage.set(key,String(value)),removeItem:key=>storage.delete(key)}};
const geometry={sodMm:100,objectToDetectorMm:150,detectorWidthMm:40,centerOffsetXMm:1,centerOffsetYMm:-2,mirrorX:true,measurement:"measured",rotationDirection:"counterclockwise"};
const draft={type:"prepare_scan",taskId:"preview-fixed",projectionCount:3,maxXraySec:120,geometry};
async function waitUntil(predicate){const deadline=Date.now()+6000;while(!predicate()){if(Date.now()>deadline)throw new Error("preview timeout");await new Promise(resolve=>setTimeout(resolve,5));}}
async function adapter(connect=true){storage.clear();const a=new WorkstationAdapter();await waitUntil(()=>a.workflow.phase==="ready");if(connect){await a.dispatch({type:"set_capture_root",savePath:"/Pictures/MicroCT"});await a.dispatch({type:"camera_connect",deviceId:"preview-camera-d7100"});await a.dispatch({type:"turntable_connect"});await a.dispatch({type:"camera_apply_properties",properties:{whiteBalance:4}});a.nativeControls.camera.multipleExposureEvidence={physicalApplicable:true,observable:true,status:"off",source:"SIMULATED TEST ONLY",reason:null};}return a;}
test("atomic preparation derives native shutter, rejects partial drafts and cancels without a task lock",async()=>{
 const a=await adapter();try{const before=await a.getSnapshot();await assert.rejects(a.dispatch({...draft,geometry:{...geometry,sodMm:0}}));const unchanged=await a.getSnapshot();assert.deepEqual(unchanged.parameters,before.parameters);assert.equal(unchanged.deviceControls.taskLock.locked,false);
 for(const taskId of ["CameraCaptures","cameracaptures","CON","bad.","bad:ads"])await assert.rejects(a.dispatch({...draft,taskId}));
 const result=await a.dispatch(draft);assert.equal(result.parameters.exposureMs,200);assert.equal(result.parameters.angleStepDeg,120);assert.deepEqual(result.workstation.scanFlow.geometry,geometry);assert.equal(result.workstation.scanFlow.prompt,null);assert.equal(result.deviceControls.taskLock.locked,false);
 await assert.rejects(a.dispatch({type:"update_scan_setup",setup:{exposureMs:125}}),/CAMERA_APPLY/);
 }finally{a.close();}
});
test("preflight and start use fresh fixed values, preserve LV and reject post-preflight drift",async()=>{
 const a=await adapter();try{await a.dispatch(draft);await a.dispatch({type:"camera_live_view_start"});const actual=structuredClone(a.nativeControls.camera.properties);await assert.rejects(a.dispatch({type:"preflight"}),/FIXED_SCAN/);assert.equal(a.nativeControls.camera.liveView.active,true);assert.deepEqual(a.nativeControls.camera.properties,actual);await a.dispatch({type:"camera_live_view_stop"});await a.dispatch({type:"preflight"});await a.dispatch({type:"home"});const ready=await a.getSnapshot();assert.equal(ready.workstation.scanFlow.prompt,null);assert.deepEqual(ready.workstation.scanFlow.geometry,geometry);
 const iso=a.nativeControls.camera.properties.iso;iso.actual=400;iso.target=400;await assert.rejects(a.dispatch({type:"start_scan"}),/MISMATCH/);assert.equal((await a.getSnapshot()).preflightPassed,false);
 }finally{a.close();}
});
test("independent automatic capture remains simulated and cannot grant a file descriptor",async()=>{
 const a=await adapter();try{const before=(await a.getSnapshot()).parameters;await a.dispatch({type:"camera_apply_properties",properties:{exposureMode:2,autoIso:1,whiteBalance:2}});const result=await a.dispatch({type:"camera_capture"});assert.equal(result.cameraScanReadiness.ready,false);assert.equal(result.imageCount,0);assert.equal(result.deviceControls.camera.lastCapturedImage,null);assert.match(result.deviceControls.camera.lastCapture,/NO HARDWARE.*not written/);assert.deepEqual(result.parameters,before);await assert.rejects(a.dispatch({type:"camera_capture_descriptor",captureId:"fake"}),/no registered NEF/);await assert.rejects(a.dispatch({type:"camera_capture",directory:"/outside"}),/storage root/);a.nativeControls.camera.properties.recordingMedia.confirmed=false;await assert.rejects(a.dispatch({type:"camera_capture"}),/Unknown/);
 }finally{a.close();}
});
test("maintenance acquires ownership before waiting, rejects occupation and Stop keeps the lock",async()=>{
 const a=await adapter();try{a.workflow.phase="paused";await assert.rejects(a.dispatch({type:"prepare_software_update"}),/OCCUPIES/);a.workflow.phase="ready";const operation=a.dispatch({type:"prepare_software_update"});let preparing=await a.getSnapshot();assert.equal(preparing.softwareUpdatePreparation.status,"preparing");assert.equal(preparing.softwareUpdatePreparation.maintenanceLocked,true);await assert.rejects(a.dispatch({type:"camera_capture"}),/MAINTENANCE/);const ready=await operation;assert.equal(ready.softwareUpdatePreparation.ready,true);assert.equal(ready.softwareUpdatePreparation.cameraCleanup,"never-owned");assert.equal(ready.mode,"developer_preview");assert.match(ready.operations.at(-1).stage,/NO HARDWARE/);await assert.rejects(a.dispatch({type:"cancel_software_update_prepare",operationId:"stale"}),/MISMATCH/);await a.dispatch({type:"stop"});const stopped=await a.getSnapshot();assert.equal(stopped.softwareUpdatePreparation.maintenanceLocked,true);assert.equal(stopped.softwareUpdatePreparation.ready,false);await assert.rejects(a.dispatch({type:"cancel_software_update_prepare",operationId:ready.softwareUpdatePreparation.operationId}),/UNCONFIRMED/);await a.dispatch({type:"camera_status"});
 }finally{a.close();}
});
test("matching cancel completes cleanup first and never reconnects camera",async()=>{
 const a=await adapter();try{const pending=a.dispatch({type:"prepare_software_update"});const id=(await a.getSnapshot()).softwareUpdatePreparation.operationId;await a.dispatch({type:"cancel_software_update_prepare",operationId:id});const result=await pending;assert.equal(result.softwareUpdatePreparation.status,"cancelled");assert.equal(result.softwareUpdatePreparation.maintenanceLocked,false);assert.equal(result.deviceControls.camera.connected,false);assert.equal(result.softwareUpdatePreparation.ready,false);
 }finally{a.close();}
});
test("taskLock.retained is the engine's record array in both production and preview",async()=>{
  // Production contract: the engine sends TaskRecord[] (an empty array is still
  // truthy in JS, so a boolean type would have been a silent UI trap).
  const engine=JSON.parse((await (await import("node:fs/promises")).readFile(new URL("./first-launch-response.jsonl", import.meta.url),"utf8")).trim()).payload;
  assert.ok(Array.isArray(engine.deviceControls.taskLock.retained),"the recorded engine snapshot must carry an array");
  const a=await adapter(false);try{const snapshot=await a.getSnapshot();assert.ok(Array.isArray(snapshot.deviceControls.taskLock.retained),"the preview must carry an array too");
    assert.deepEqual(Object.keys(snapshot.deviceControls.taskLock).sort(),["locked","reason","retained","savePath","status","taskId"]);
  }finally{a.close();}
});
test("retained task data is loaded before UI edits and legacy evidence cannot resume",async()=>{
 storage.clear();storage.set("micro-ct-workstation.checkpoint",JSON.stringify({taskId:"retained",savePath:"/old/root",projectionCount:2,exposureMs:200,maxXraySec:120,view:1,angleDeg:0,ts:new Date().toISOString()}));const a=new WorkstationAdapter();try{await waitUntil(()=>a.workflow.phase==="ready");const snapshot=await a.getSnapshot();assert.equal(snapshot.deviceControls.taskLock.locked,true);assert.equal(snapshot.parameters.savePath,"/old/root");await assert.rejects(a.dispatch({type:"set_capture_root",savePath:"/new/root"}),/OCCUPIES/);await a.dispatch({type:"camera_connect",deviceId:"preview-camera-d7100"});assert.equal(a.workflow.params.savePath,"/old/root");await assert.rejects(a.dispatch({type:"preflight",restore:true,confirmUsbAutoShutdownRelease:true}),/LEGACY/);await assert.rejects(a.dispatch({type:"prepare_software_update"}),/OCCUPIES/);
 }finally{a.close();storage.clear();}
});
test("reconstruction source uses the shared aggregate with stable event IDs",async()=>{const a=await adapter(false);try{await assert.rejects(a.dispatch({type:"start_reconstruction",method:"fdk"}),/no acquired NEF/);const first=await a.getSnapshot();const event=first.workstation.consoleLogs.find(entry=>entry.source==="reconstruction");assert.ok(event);const second=await a.getSnapshot();assert.deepEqual(second.workstation.consoleLogs,first.workstation.consoleLogs);}finally{a.close();}});

test("realistic unobservable multiple exposure blocks fixed scans but independent one-output remains simulated",async()=>{const a=await adapter();try{a.nativeControls.camera.multipleExposureEvidence={physicalApplicable:true,observable:false,status:"unknown",source:"none",reason:"Model state is unobservable"};const snapshot=await a.getSnapshot();assert.equal(snapshot.cameraScanReadiness.ready,false);assert.ok(snapshot.cameraScanReadiness.issues.some(issue=>issue.key==="multipleExposure"));await a.dispatch(draft);await assert.rejects(a.dispatch({type:"preflight"}),/FIXED_SCAN/);const photo=await a.dispatch({type:"camera_capture"});assert.equal(photo.deviceControls.camera.lastCapturedImage,null);assert.match(photo.deviceControls.camera.lastCapture,/not written/);}finally{a.close();}});
test("operator OFF attestation is session-bound in the preview and never rewrites machine evidence",async()=>{
 const a=await adapter();
 try{
  const machine={physicalApplicable:true,observable:false,status:"unknown",source:"none",reason:"Model state is unobservable"};
  a.nativeControls.camera.multipleExposureEvidence=structuredClone(machine);
  const blocked=await a.getSnapshot();assert.equal(blocked.cameraScanReadiness.ready,false);assert.equal(blocked.deviceControls.camera.multipleExposureOperator.state,"none");
  const confirmed=await a.dispatch({type:"camera_confirm_multiple_exposure_off",confirmed:true,operator:"test-operator"});
  assert.equal(confirmed.deviceControls.camera.multipleExposureOperator.state,"confirmed");
  assert.equal(confirmed.deviceControls.camera.multipleExposureOperator.valid,true);
  assert.equal(confirmed.deviceControls.camera.multipleExposureOperator.policyVersion,1);
  assert.equal(confirmed.cameraScanReadiness.ready,true,JSON.stringify(confirmed.cameraScanReadiness.issues));
  // The machine state is untouched: the attestation is not device truth.
  assert.deepEqual(confirmed.deviceControls.camera.multipleExposureEvidence,machine);
  assert.equal(confirmed.deviceControls.camera.multipleExposureOperator.machineStatus,"unknown");
  assert.equal(confirmed.deviceControls.camera.multipleExposureOperator.machineObservable,false);
  // Any changed confirmed parameter expires the session-bound attestation.
  const stale=await a.dispatch({type:"camera_apply_properties",properties:{whiteBalance:5}});
  assert.equal(stale.deviceControls.camera.multipleExposureOperator.state,"stale");
  assert.equal(stale.cameraScanReadiness.ready,false);
  // A new session cannot inherit the previous attestation.
  await a.dispatch({type:"camera_disconnect"});
  await a.dispatch({type:"camera_connect",deviceId:"preview-camera-d7100"});
  const reconnected=await a.getSnapshot();
  assert.equal(reconnected.deviceControls.camera.multipleExposureOperator.state,"none");
  assert.equal(reconnected.cameraScanReadiness.ready,false);
  // An explicit withdrawal is not a confirmation.
  await a.dispatch({type:"camera_confirm_multiple_exposure_off",confirmed:true,operator:"test-operator"});
  const withdrawn=await a.dispatch({type:"camera_confirm_multiple_exposure_off",confirmed:false});
  assert.equal(withdrawn.deviceControls.camera.multipleExposureOperator.state,"none");
  assert.equal(withdrawn.cameraScanReadiness.ready,false);
 }finally{a.close();}
});
