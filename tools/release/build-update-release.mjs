/** Build one immutable Windows x64 MSI; compiler EXEs are never distribution outputs. */
import {spawnSync,execFileSync} from 'node:child_process';
import {copyFileSync,existsSync,lstatSync,mkdirSync,readFileSync,readdirSync,realpathSync,rmSync,statSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateReleaseConfiguration,workspaceApplicationVersion} from '../lib/release-version.mjs';
const root=resolve(import.meta.dirname,'../..');
const version=JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version;
const output=join(root,'.cache/tests/release',version);
const delivery=join(root,'release');
const filename='micro-ct-workstation_'+version+'_windows_x86_64.msi';
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
function run(exe,args){const child=spawnSync(exe,args,{cwd:root,stdio:'inherit',env:{...process.env,TAURI_SIGNING_PRIVATE_KEY:undefined,TAURI_SIGNING_PRIVATE_KEY_PATH:undefined,TAURI_SIGNING_PRIVATE_KEY_PASSWORD:undefined}});if(child.status!==0)throw Error(exe+' exited '+child.status);}
if(process.platform!=='win32'||process.arch!=='x64')throw Error('Build the Windows x64 installer on Windows x64');
if(JSON.parse(readFileSync(join(root,'node_modules/@tauri-apps/cli/package.json'),'utf8')).version!=='2.12.1')throw Error('CLI must be exactly 2.12.1');
const config=JSON.parse(readFileSync(join(root,'backend/src-tauri/tauri.conf.json'),'utf8'));
const ledger=JSON.parse(readFileSync(join(root,'tools/release/msi-version-ledger.json'),'utf8'));
validateReleaseConfiguration(version,workspaceApplicationVersion(readFileSync(join(root,'backend/Cargo.toml'),'utf8')),config,ledger);
if(config.bundle.targets.length!==1||config.bundle.targets[0]!=='msi')throw Error('Only MSI delivery is supported');
if(existsSync(join(output,filename)))throw Error('Refusing to overwrite the final installer; allocate a new version');
mkdirSync(output,{recursive:true});
run(process.execPath,[join(root,'tools/release/collect-release-licenses.mjs'),output]);
run(process.execPath,[join(root,'tools/release/prepare-installer-ui.mjs')]);
// Bundling stamps the installed app with MSI identity independently of the internal build executable.
run(process.execPath,[join(root,'tools/build/run-tauri.mjs'),'build','--bundles','msi']);
const directory=join(root,'.cache/cargo/release/bundle/msi');
const files=readdirSync(directory).filter(name=>name.endsWith('.msi')&&name.includes(version));
if(files.length!==1)throw Error('Expected exactly one current-version MSI');
run('pwsh',['-NoProfile','-File',join(root,'tools/release/verify-installer-ui.ps1'),'-Path',join(directory,files[0])]);
copyFileSync(join(directory,files[0]),join(output,filename));
const sourceCommit=execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim();
const dirty=Boolean(execFileSync('git',['status','--porcelain'],{cwd:root,encoding:'utf8'}).trim());
const sourceFiles=execFileSync('git',['ls-files','--cached','--others','--exclude-standard','-z'],{cwd:root,encoding:'utf8'}).split('\0').filter(Boolean).filter(file=>existsSync(join(root,file))).sort();
const sourceTreeSha256=sha(Buffer.from(sourceFiles.map(file=>sha(readFileSync(join(root,file)))+'  '+file).join('\n')));
writeFileSync(join(output,'build-info.json'),JSON.stringify({schemaVersion:2,version,cliVersion:'2.12.1',builtAt:new Date().toISOString(),sourceCommit,dirty,sourceTreeSha256,validationScope:'offline-only',hardwareCertified:false,supportedWindows:['Windows 10 x64','Windows 11 x64'],artifacts:[{file:filename,size:statSync(join(output,filename)).size,sha256:sha(readFileSync(join(output,filename)))}],engineSha256:sha(readFileSync(join(root,'backend/src-tauri/binaries/ct-engine-x86_64-pc-windows-msvc.exe')))},null,2)+'\n');
// The public delivery directory contains exactly the latest installer. Evidence,
// licenses and signatures stay in the versioned build cache and inside the MSI.
mkdirSync(delivery,{recursive:true});
if(lstatSync(delivery).isSymbolicLink()||realpathSync(delivery)!==delivery)throw Error('Release directory must be the local generated directory');
for(const entry of readdirSync(delivery)){
  const path=join(delivery,entry);
  if(lstatSync(path).isSymbolicLink())throw Error('Refusing to remove a release link');
}
for(const entry of readdirSync(delivery))rmSync(join(delivery,entry),{recursive:true,force:true});
copyFileSync(join(output,filename),join(delivery,filename));
console.log('PASS latest Windows MSI only in release/; evidence staged in .cache/tests/release/'+version);
