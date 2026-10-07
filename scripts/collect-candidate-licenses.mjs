/** Collect declared locked dependency licenses and available attribution texts.
 * Inputs are Cargo/npm metadata and installed source notices; outputs are only
 * generated candidate artifacts. This tool never discovers or opens devices.
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, relative } from "node:path";
import { createHash } from "node:crypto";

const root = resolve(import.meta.dirname, "..");
const output = resolve(root, process.argv[2] || "release/native-device-controls-offline");
const releaseVersion=JSON.parse(readFileSync(join(root,"package.json"),"utf8")).version;
if (output !== join(root,"release/native-device-controls-offline") && output !== join(root,"release",releaseVersion)) throw new Error("License output must be the offline candidate or current version release directory");
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
const cargo = JSON.parse(execFileSync("cargo",["metadata","--format-version","1","--locked","--offline","--filter-platform","x86_64-pc-windows-msvc"],{cwd:root,encoding:"utf8",maxBuffer:32*1024*1024}));
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
const nikon = join(root,"crates/ct-engine/src/devices/camera/THIRD-PARTY-NOTICES.md");
copyFileSync(nikon,join(output,"THIRD-PARTY-NOTICES.md"));
records.sort((a,b) => `${a.kind}/${a.name}/${a.version}`.localeCompare(`${b.kind}/${b.name}/${b.version}`));
writeFileSync(join(output,"third-party-licenses.json"),JSON.stringify({schemaVersion:1,source:"locked Cargo metadata and installed npm package declarations",packages:records},null,2)+"\n");
console.log(`PASS collected ${records.length} declared dependencies and their available license/notice texts`);
