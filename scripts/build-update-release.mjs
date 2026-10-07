/** Fresh local distribution build. Always preserves the formal portable entry. */
import {spawnSync,execFileSync} from 'node:child_process';
import {copyFileSync,existsSync,mkdirSync,readFileSync,readdirSync,statSync,writeFileSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {validateReleaseConfiguration,workspaceApplicationVersion} from './lib/release-version.mjs';
const root=resolve(import.meta.dirname,'..');
const packageJson=JSON.parse(readFileSync(join(root,'package.json'),'utf8'));
const version=packageJson.version;
const cli=join(root,'node_modules/@tauri-apps/cli/tauri.js');
const output=join(root,'release',version);
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const formal=join(root,'portable-release/micro-ct-workstation-portable.exe');
const original=existsSync(formal)?sha(readFileSync(formal)):null;
function run(exe,args){const child=spawnSync(exe,args,{cwd:root,stdio:'inherit',env:{...process.env,TAURI_SIGNING_PRIVATE_KEY:undefined,TAURI_SIGNING_PRIVATE_KEY_PATH:undefined,TAURI_SIGNING_PRIVATE_KEY_PASSWORD:undefined}});if(child.status!==0)throw Error(`${exe} exited ${child.status}`);}
if(process.platform!=='win32')throw Error('Windows distribution must be built on Windows');
if(JSON.parse(readFileSync(join(root,'node_modules/@tauri-apps/cli/package.json'),'utf8')).version!=='2.12.1')throw Error('CLI must be exactly 2.12.1');
const config=JSON.parse(readFileSync(join(root,'src-tauri/tauri.conf.json'),'utf8'));
const ledger=JSON.parse(readFileSync(join(root,'scripts/msi-version-ledger.json'),'utf8'));
validateReleaseConfiguration(version,workspaceApplicationVersion(readFileSync(join(root,'Cargo.toml'),'utf8')),config,ledger);
mkdirSync(output,{recursive:true});
function copyFinal(source,name){const destination=join(output,name);if(existsSync(destination))throw Error(`Refusing to overwrite ${name}`);copyFileSync(source,destination);return destination;}
const prefix=`micro-ct-workstation_${version}_windows_x86_64`;
try{
  run(process.execPath,[cli,'build','--no-bundle']);
  const app=join(root,'target/release/ct-workstation.exe');
  const bytes=readFileSync(app);
  if(!bytes.includes(Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK')))throw Error('No-bundle EXE lacks portable identity marker');
  copyFinal(app,`${prefix}-portable.exe`);
  // Save the unmarked EXE first: bundlers stamp their packaged app independently.
  run(process.execPath,[cli,'bundle','--bundles','nsis,msi']);
  for(const kind of ['nsis','msi']){
    const directory=join(root,'target/release/bundle',kind);
    const suffix=kind==='nsis'?'.exe':'.msi';
    const files=readdirSync(directory).filter(name=>name.endsWith(suffix)&&name.includes(version));
    if(files.length!==1)throw Error(`Expected one fresh ${kind} artifact; got ${files.length}`);
    const source=join(directory,files[0]);
    copyFinal(source,kind==='nsis'?`${prefix}-setup.exe`:`${prefix}.msi`);
  }
  const sourceCommit=execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim();
  const dirty=Boolean(execFileSync('git',['status','--porcelain'],{cwd:root,encoding:'utf8'}).trim());
  const sourceFiles=execFileSync('git',['ls-files','--cached','--others','--exclude-standard','-z'],{cwd:root,encoding:'utf8'}).split('\0').filter(Boolean).filter(file=>existsSync(join(root,file))).sort();
  const sourceTreeSha256=sha(Buffer.from(sourceFiles.map(file=>`${sha(readFileSync(join(root,file)))}  ${file}`).join('\n')));
  writeFileSync(join(output,'build-info.json'),JSON.stringify({schemaVersion:1,version,cliVersion:'2.12.1',builtAt:new Date().toISOString(),sourceCommit,dirty,sourceTreeSha256,hardwareAcceptance:'仅离线验证',formalPortableSha256:original,artifacts:readdirSync(output).filter(f=>/\.(exe|msi)$/.test(f)).map(file=>({file,size:statSync(join(output,file)).size,sha256:sha(readFileSync(join(output,file)))}))},null,2)+'\n');
}finally{if(original!==null&&sha(readFileSync(formal))!==original)throw Error('Formal portable entry changed');}
console.log(`PASS local unsigned distributions staged at release/${version}; sign only final bytes after identity inspection`);
