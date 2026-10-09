/** Signs immutable staged artifacts, manifest and ZIP_STORED offline containers. */
import {spawnSync,execFileSync} from 'node:child_process';
import {existsSync,mkdirSync,readFileSync,writeFileSync,statSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {parseReleaseVersion,validateReleaseConfiguration,workspaceApplicationVersion} from '../lib/release-version.mjs';
const root=resolve(import.meta.dirname, "../..");
const version=JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version;
const output=join(root,'.cache/tests/release',version);
const prefix=`micro-ct-workstation_${version}_windows_x86_64`;
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const kinds=['msi'];
const names={msi:`${prefix}.msi`};
if(existsSync(join(output,'release-manifest.json'))||kinds.some(kind=>existsSync(join(output,names[kind]+'.sig'))))throw Error('Signed material already exists; refuse to overwrite immutable signatures');
function run(exe,args,options={}){const result=spawnSync(exe,args,{cwd:root,stdio:'inherit',...options});if(result.status!==0)throw Error(`${exe} failed ${result.status}`);}
function sign(name){run('pwsh',['-NoProfile','-File',join(root,'tools/release/update-sign.ps1'),'-Action','Sign','-File',join(output,name),'-Version',version]);}
const ledger=JSON.parse(readFileSync(join(root,'tools/release/msi-version-ledger.json'),'utf8'));
const configuration=JSON.parse(readFileSync(join(root,'backend/src-tauri/tauri.conf.json'),'utf8'));
const mapping=validateReleaseConfiguration(version,workspaceApplicationVersion(readFileSync(join(root,'backend/Cargo.toml'),'utf8')),configuration,ledger);
const actual=JSON.parse(execFileSync('pwsh',['-NoProfile','-File',join(root,'tools/release/inspect-msi.ps1'),'-Path',join(output,names.msi)],{cwd:root,encoding:'utf8'}));
if(actual.productVersion!==mapping.productVersion||actual.upgradeCode.replace(/[{}]/g,'').toUpperCase()!==ledger.upgradeCode.toUpperCase()||!/^\{[0-9A-F-]{36}\}$/i.test(actual.productCode)||!actual.template.startsWith('x64;'))throw Error('Actual MSI Property/Summary table differs from ledger/architecture');
writeFileSync(join(output,'msi-identity.json'),JSON.stringify(actual,null,2)+'\n');
const identities=JSON.parse(readFileSync(join(output,'bundle-identities.json'),'utf8'));
const identity=identities.msi;
if(identity?.kind!=='msi'||identity.marker!=='__TAURI_BUNDLE_TYPE_VAR_MSI'||identity.artifactSha256!==sha(readFileSync(join(output,names.msi)))||!/^[0-9a-f]{64}$/.test(identity.applicationExeSha256??'')||!/^[0-9a-f]{64}$/.test(identity.engineSha256??''))throw Error('Missing verified MSI application/engine identities');
const sourceCommit=execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim();
for(const kind of kinds){if(!existsSync(join(output,names[kind])))throw Error(`Missing ${kind}`);sign(names[kind]);}
const manifest={schemaVersion:1,appId:'edu.lzu.microct.workstation',version,channel:parseReleaseVersion(version).prerelease.length?'candidate':'stable',publishedAt:new Date().toISOString(),sourceCommit,releaseNotes:'Windows installer with installation folder, optional shortcuts and repair. Device acceptance is tracked separately.',artifacts:kinds.map(kind=>{const fileName=names[kind];return{target:'windows-x86_64',kind,fileName,size:statSync(join(output,fileName)).size,sha256:sha(readFileSync(join(output,fileName))),signature:readFileSync(join(output,fileName+'.sig'),'utf8').trim(),url:`https://github.com/Holy1one/Micro-CT-Workstation/releases/download/v${version}/${fileName}`,...(kind==='msi'?{installerProductVersion:mapping.productVersion}:{})};})};
const manifestPath=join(output,'release-manifest.json');
if(existsSync(manifestPath))throw Error('Signed release manifest already exists; preserve immutable bytes');
writeFileSync(manifestPath,JSON.stringify(manifest,null,2)+'\n');sign('release-manifest.json');
const checks=manifest.artifacts.map(a=>`${a.sha256}  ${a.fileName}`);checks.push(`${sha(readFileSync(manifestPath))}  release-manifest.json`);writeFileSync(join(output,'SHA256SUMS'),checks.join('\n')+'\n');
const verifier=join(root,'.cache/cargo/release/ct-workstation.exe');
for(const kind of kinds)run(verifier,['--verify-update-package',output,manifest.channel,kind,'0.8.2']);
// ZIP_STORED writer is deliberately small and uses only exact basenames.
let crcTable;function crc32(bytes){crcTable??=Array.from({length:256},(_,n)=>{for(let i=0;i<8;i++)n=n&1?0xedb88320^(n>>>1):n>>>1;return n>>>0;});let crc=0xffffffff;for(const b of bytes)crc=crcTable[(crc^b)&255]^(crc>>>8);return(crc^0xffffffff)>>>0;}
function zipStored(files){let offset=0;const locals=[];const central=[];for(const name of files){const bytes=readFileSync(join(output,name));const filename=Buffer.from(name);const crc=crc32(bytes);const header=Buffer.alloc(30);header.writeUInt32LE(0x04034b50);header.writeUInt16LE(20,4);header.writeUInt32LE(crc,14);header.writeUInt32LE(bytes.length,18);header.writeUInt32LE(bytes.length,22);header.writeUInt16LE(filename.length,26);const cd=Buffer.alloc(46);cd.writeUInt32LE(0x02014b50);cd.writeUInt16LE(20,4);cd.writeUInt16LE(20,6);cd.writeUInt32LE(crc,16);cd.writeUInt32LE(bytes.length,20);cd.writeUInt32LE(bytes.length,24);cd.writeUInt16LE(filename.length,28);cd.writeUInt32LE(offset,42);locals.push(header,filename,bytes);central.push(cd,filename);offset+=header.length+filename.length+bytes.length;}const directory=Buffer.concat(central);const end=Buffer.alloc(22);end.writeUInt32LE(0x06054b50);end.writeUInt16LE(files.length,8);end.writeUInt16LE(files.length,10);end.writeUInt32LE(directory.length,12);end.writeUInt32LE(offset,16);return Buffer.concat([...locals,directory,end]);}
const verificationDirectory=join(root,'.cache/tests/release',version);mkdirSync(verificationDirectory,{recursive:true});
for(const kind of kinds){const files=['release-manifest.json','release-manifest.json.sig',names[kind],names[kind]+'.sig',...['LICENSE.txt','THIRD-PARTY-NOTICES.md','third-party-licenses.json'].filter(name=>existsSync(join(output,name))&&statSync(join(output,name)).size<=256*1024)];const filename=`${prefix}_${kind}-verification.zip`;writeFileSync(join(verificationDirectory,filename),zipStored(files),{flag:'wx'});run(verifier,['--verify-offline-update',join(verificationDirectory,filename),kind,'0.8.2']);}
console.log(`PASS all final artifacts and manifest signed; MSI update container created. No publication performed.`);
