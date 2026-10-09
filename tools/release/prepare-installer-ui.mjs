/** Build the embedded native folder picker with the existing Windows MSVC SDK. */
import { execFileSync, spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '../..');
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
const output = join(root, '.cache/tests/release', version, 'installer-ui');
mkdirSync(output, { recursive: true });
if (process.platform !== 'win32' || process.arch !== 'x64') throw Error('Installer UI requires Windows x64');
const vswhere = join(process.env['ProgramFiles(x86)'], 'Microsoft Visual Studio/Installer/vswhere.exe');
const installation = execFileSync(vswhere, ['-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'], { encoding: 'utf8', windowsHide: true }).trim();
if (!installation || /["\r\n]/.test(installation)) throw Error('A Windows MSVC build environment is required');
const vcvars = join(installation, 'VC/Auxiliary/Build/vcvars64.bat');
const environment = spawnSync('cmd.exe', ['/d', '/s', '/c', `"call "${vcvars}" >nul && set"`], { encoding: 'utf8', windowsHide: true, windowsVerbatimArguments: true });
if (environment.error || environment.status !== 0) throw Error('Unable to initialize the MSVC environment');
const variables = environment.stdout;
const env = { ...process.env };
for (const line of variables.split(/\r?\n/)) {
  const separator = line.indexOf('=');
  if (separator > 0) env[line.slice(0, separator)] = line.slice(separator + 1);
}
const source = join(root, 'tools/release/windows-folder-picker.cpp');
const dll = join(output, 'installer-folder-picker.dll');
const args = ['/nologo', '/LD', '/MT', '/EHsc', '/std:c++17', '/W4', '/WX', '/O2', '/DUNICODE', '/D_UNICODE', '/DWIN32_LEAN_AND_MEAN', source, '/Fo' + join(output, 'installer-folder-picker.obj'), '/Fe' + dll, '/link', 'msi.lib', 'ole32.lib', 'shell32.lib', 'user32.lib', 'uuid.lib'];
const compile = spawnSync('cl.exe', args, { cwd: output, env, stdio: 'inherit', windowsHide: true });
if (compile.error || compile.status !== 0) throw Error('Native installer UI build failed: ' + (compile.error ?? compile.status));
const gen = join(root, 'backend/src-tauri/gen');
mkdirSync(gen, { recursive: true });
copyFileSync(dll, join(gen, 'installer-folder-picker.dll'));
const xmlPath = join(gen, 'installer-folder-picker.dll').replaceAll('&', '&amp;').replaceAll('"', '&quot;');
writeFileSync(join(gen, 'installer-folder-picker.wxs'), `<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi"><Fragment><Binary Id="NativeFolderPicker" SourceFile="${xmlPath}" /></Fragment></Wix>\n`);
const sha = path => createHash('sha256').update(readFileSync(path)).digest('hex');
writeFileSync(join(output, 'identity.json'), JSON.stringify({ version, sourceSha256: sha(source), dllSha256: sha(dll), compiler: 'MSVC x64', scope: 'Immediate folder selection UI only' }, null, 2) + '\n');
console.log('PASS native installer folder picker built and staged');
