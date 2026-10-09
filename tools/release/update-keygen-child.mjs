// Password exists only in this short-lived child environment and CLI memory.
// The official CLI's run API avoids placing it in Windows process argv.
import { createRequire } from 'node:module';
const require=createRequire(import.meta.url);
const {run}=require('@tauri-apps/cli/main.js');
const version=require('@tauri-apps/cli/package.json').version;
if(version!=='2.12.1'||process.argv.length!==3||!process.env.MICROCT_KEYGEN_PASSWORD)process.exit(1);
const password=process.env.MICROCT_KEYGEN_PASSWORD;
delete process.env.MICROCT_KEYGEN_PASSWORD;
delete process.env.CI;
try{await run(['signer','generate','-w',process.argv[2],'-p',password],'microct-protected-keygen');}catch{process.exit(1);}
