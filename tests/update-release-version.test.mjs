/** Executes release gates without compiling/signing/installing a package. */
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { compareReleaseVersions, parseReleaseVersion, validateVersionLedger, validateReleaseConfiguration, workspaceApplicationVersion } from '../scripts/lib/release-version.mjs';
const root = new URL('../', import.meta.url);
const readJson = file => JSON.parse(readFileSync(new URL(file, root), 'utf8'));
const ledger = readJson('scripts/msi-version-ledger.json');
test('stable 0.9.5 versions and MSI allocation match while prior signed allocations remain immutable', () => {
  const packageVersion = readJson('package.json').version;
  const workspaceVersion = workspaceApplicationVersion(readFileSync(new URL('Cargo.toml', root), 'utf8'));
  const configuration = readJson('src-tauri/tauri.conf.json');
  assert.equal(packageVersion, '0.9.5');
  assert.equal(readJson('package-lock.json').version, packageVersion);
  assert.equal(readJson('package-lock.json').packages[''].version, packageVersion);
  assert.deepEqual(validateReleaseConfiguration(packageVersion, workspaceVersion, configuration, ledger), { appVersion: '0.9.5', productVersion: '0.9.5' });
  assert.deepEqual(ledger.versions[0], { appVersion: '0.9.0-rc.1', productVersion: '0.9.1' });
  assert.deepEqual(ledger.versions[1], { appVersion: '0.9.0-rc.2', productVersion: '0.9.2' });
  assert.deepEqual(ledger.versions[2], { appVersion: '0.9.0-rc.3', productVersion: '0.9.3' });
});
test('canonical SemVer rejects aliases, leading zeros, empty identifiers and trailing junk', () => {
  for (const value of ['v0.9.0', '0.9', '00.9.0', '0.9.0-rc.02', '0.9.0-', '0.9.0+.', '0.9.0 rc2', '0.9.0\n', '0.9.0-rc..2']) assert.throws(() => parseReleaseVersion(value));
  assert.equal(parseReleaseVersion('0.9.0-rc.2+build.01').value, '0.9.0-rc.2+build.01');
});
test('SemVer precedence orders rc numbers and stable correctly without build metadata', () => {
  assert.equal(compareReleaseVersions('0.9.0-rc.10', '0.9.0-rc.2'), 1);
  assert.equal(compareReleaseVersions('0.9.0', '0.9.0-rc.2'), 1);
  assert.equal(compareReleaseVersions('0.9.0+new', '0.9.0+old'), 0);
  assert.equal(compareReleaseVersions('1.0.0-alpha.1', '1.0.0-alpha.beta'), -1);
});
test('ledger blocks duplicate, decreased and remapped releases and MSI sequence overflow', () => {
  for (const mutate of [
    value => value.versions.push({ ...value.versions[0] }),
    value => value.versions.reverse(),
    value => { value.versions[1].productVersion = '0.9.1'; },
    value => { value.versions[1].productVersion = '0.9.65536'; },
    value => { value.versions[1].productVersion = '0.8.2'; },
    value => { value.versions[1].appVersion = '0.9.0-rc.1+new'; },
  ]) { const invalid = structuredClone(ledger); mutate(invalid); assert.throws(() => validateVersionLedger(invalid, '0.9.0-rc.2')); }
  assert.throws(() => validateVersionLedger(ledger, '0.9.0-rc.4'));
});
test('release configuration rejects version drift and missing or enabled installer downgrade setting', () => {
  const configuration = readJson('src-tauri/tauri.conf.json');
  assert.throws(() => validateReleaseConfiguration('0.9.0-rc.2', '0.9.0-rc.1', configuration, ledger));
  for (const downgrade of [undefined, true]) { const invalid = structuredClone(configuration); invalid.bundle.windows.allowDowngrades = downgrade; assert.throws(() => validateReleaseConfiguration('0.9.5', '0.9.5', invalid, ledger)); }
});
