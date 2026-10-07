/** Verify actual unpacked/installed EXEs against the portable marker offset. */
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
const root=resolve(import.meta.dirname,'..');
const version=JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version;
const output=join(root,'release',version);
const prefix=`micro-ct-workstation_${version}_windows_x86_64`;
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const portable=readFileSync(join(output,`${prefix}-portable.exe`));
const unknown=Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
const offset=portable.indexOf(unknown);
if(offset<0||portable.indexOf(unknown,offset+1)>=0)throw Error('Portable marker location is ambiguous');
if(process.argv.length!==4)throw Error('Pass actual NSIS and MSI unpacked/installed application EXE paths');
const identities={};
for(const [index,kind] of ['nsis','msi'].entries()){
  const exe=resolve(process.argv[index+2]);
  const bytes=readFileSync(exe);
  const marker=`__TAURI_BUNDLE_TYPE_VAR_${kind==='nsis'?'NSS':'MSI'}`;
  if(bytes.subarray(offset,offset+unknown.length).toString()!==marker)throw Error(`Actual ${kind} app has the wrong bundle marker`);
  const restored=Buffer.from(bytes);unknown.copy(restored,offset);
  if(!restored.equals(portable))throw Error(`${kind} packaged app differs from the compiled portable bytes beyond its bundle marker; inspect any Authenticode change explicitly`);
  const artifact=join(output,kind==='nsis'?`${prefix}-setup.exe`:`${prefix}.msi`);
  identities[kind]={kind,marker,markerOffset:offset,artifactSha256:sha(readFileSync(artifact)),installedExeSha256:sha(bytes),portableSha256:sha(portable)};
}
writeFileSync(join(output,'bundle-identities.json'),JSON.stringify(identities,null,2)+'\n',{flag:'wx'});
console.log('PASS actual packaged EXEs match portable bytes and independent bundle markers');
