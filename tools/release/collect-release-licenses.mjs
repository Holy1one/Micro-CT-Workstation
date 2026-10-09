/** Collect declared locked dependency licenses and available attribution texts.
 * Inputs are Cargo/npm metadata and installed source notices; outputs are only
 * generated release artifacts. This tool never discovers or opens devices.
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, relative } from "node:path";
import { createHash } from "node:crypto";

const root = resolve(import.meta.dirname, "../..");
const releaseVersion=JSON.parse(readFileSync(join(root,"package.json"),"utf8")).version;
const output = resolve(root, process.argv[2] || `.cache/tests/release/${releaseVersion}`);
if (output !== join(root,".cache/tests/release",releaseVersion)) throw new Error("License output must be the current version build evidence directory");
const notices = join(output,"licenses"); mkdirSync(notices,{recursive:true});
const safe = (value) => value.replace(/[^a-zA-Z0-9_.-]/g,"_");
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const records = [];
function collect(kind,name,version,license,repository,directory,explicitLicenseFile) {
  const files = new Set(readdirSync(directory).filter(file => /^(?:LICEN[CS]E|COPYING|NOTICE)(?:[-_.].*)?$/i.test(file)).map(file => join(directory,file)));
  if (explicitLicenseFile && existsSync(explicitLicenseFile)) files.add(explicitLicenseFile);
  const copied = [];
  for (const file of files) {
    if (!statSync(file).isFile() || statSync(file).size > 512*1024) continue;
    const target = join(notices,`${safe(kind)}-${safe(name)}-${safe(version)}-${safe(file.split(/[\\/]/).at(-1))}`);
    copyFileSync(file,target); copied.push({file:relative(output,target).replaceAll("\\","/"),sha256:digest(readFileSync(target))});
  }
  records.push({kind,name,version,license:license || "Declared license unavailable",repository:repository || null,attributionFiles:copied});
}
const cargo = JSON.parse(execFileSync("cargo",["metadata","--manifest-path","backend/Cargo.toml","--format-version","1","--locked","--offline","--filter-platform","x86_64-pc-windows-msvc"],{cwd:root,encoding:"utf8",maxBuffer:32*1024*1024}));
for (const pkg of cargo.packages) {
  if (cargo.workspace_members.includes(pkg.id)) continue;
  collect("rust",pkg.name,pkg.version,pkg.license,pkg.repository,dirname(pkg.manifest_path),pkg.license_file);
}
const lock = JSON.parse(readFileSync(join(root,"package-lock.json"),"utf8"));
for (const [path,pkg] of Object.entries(lock.packages)) {
  if (!path.startsWith("node_modules/")) continue;
  const directory = join(root,path);
  if (!existsSync(join(directory,"package.json"))) continue;
  const manifest = JSON.parse(readFileSync(join(directory,"package.json"),"utf8"));
  collect("npm",manifest.name,pkg.version,manifest.license || pkg.license,
    typeof manifest.repository === "string" ? manifest.repository : manifest.repository?.url,directory);
}
const nikon = join(root,"backend/ct-engine/src/devices/camera/THIRD-PARTY-NOTICES.md");
copyFileSync(nikon,join(output,"THIRD-PARTY-NOTICES.md"));
records.sort((a,b) => `${a.kind}/${a.name}/${a.version}`.localeCompare(`${b.kind}/${b.name}/${b.version}`));
writeFileSync(join(output,"third-party-licenses.json"),JSON.stringify({schemaVersion:1,source:"locked Cargo metadata and installed npm package declarations",packages:records},null,2)+"\n");
// Current-user MSI components require HKCU registry key paths. Author those
// explicitly instead of using per-machine resource components from the bundler.
const xml = value => value.replaceAll('&','&amp;').replaceAll('"','&quot;').replaceAll('<','&lt;');
const componentGuid = name => {
  const value = digest(Buffer.from('MicroCT/MSI/license/'+name));
  return [value.slice(0,8),value.slice(8,12),value.slice(12,16),value.slice(16,20),value.slice(20,32)].join('-');
};
const files = [join(output,'THIRD-PARTY-NOTICES.md'),join(output,'third-party-licenses.json'),
  ...readdirSync(notices).map(name=>join(notices,name)).filter(path=>statSync(path).isFile())];
const components = files.map(path => {
  const name = path.split(/[\\/]/).at(-1);
  const id = 'License_' + digest(Buffer.from(name)).slice(0,24);
  return `<Component Id="${id}" Guid="${componentGuid(name)}" Win64="yes"><File Id="File_${id}" Source="${xml(path)}" Name="${xml(name)}" KeyPath="no" /><RegistryValue Root="HKCU" Key="Software\\lzu\\Micro-CT Workstation\\Licenses" Name="${id}" Type="integer" Value="1" KeyPath="yes" /></Component>`;
}).join('\n');
const gen = join(root,'backend/src-tauri/gen');mkdirSync(gen,{recursive:true});
writeFileSync(join(gen,'installer-licenses.wxs'),`<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi"><Fragment><DirectoryRef Id="INSTALLDIR"><Directory Id="LicenseFolder" Name="licenses" /></DirectoryRef><DirectoryRef Id="LicenseFolder"><Component Id="LicenseDirectory" Guid="*" Win64="yes"><RemoveFolder Id="RemoveLicenseDirectory" On="uninstall" /><RegistryValue Root="HKCU" Key="Software\\lzu\\Micro-CT Workstation" Name="Licenses" Type="integer" Value="1" KeyPath="yes" /></Component>${components}</DirectoryRef><ComponentGroup Id="LicenseFiles"><ComponentRef Id="LicenseDirectory" />${files.map(path=>'<ComponentRef Id="License_'+digest(Buffer.from(path.split(/[\\/]/).at(-1))).slice(0,24)+'" />').join('')}</ComponentGroup></Fragment></Wix>\n`);
console.log(`PASS collected ${records.length} declared dependencies and their available license/notice texts`);
