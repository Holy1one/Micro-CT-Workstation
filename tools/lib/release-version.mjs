/** Canonical SemVer and the public, monotonic Windows installer ledger. */
const semanticVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
export function parseReleaseVersion(value) {
  if (typeof value !== 'string') throw Error('Application version must be a canonical SemVer string');
  const match = semanticVersion.exec(value);
  if (!match) throw Error(`Invalid canonical application SemVer: ${value}`);
  const prerelease = match[4]?.split('.') ?? [];
  if (prerelease.some(identifier => /^\d+$/.test(identifier) && identifier.length > 1 && identifier.startsWith('0'))) throw Error('Numeric SemVer prerelease identifiers cannot have leading zeros');
  return { value, core: match.slice(1, 4).map(BigInt), prerelease, build: match[5]?.split('.') ?? [] };
}
export function compareReleaseVersions(leftValue, rightValue) {
  const left = parseReleaseVersion(leftValue), right = parseReleaseVersion(rightValue);
  for (let index = 0; index < 3; index++) if (left.core[index] !== right.core[index]) return left.core[index] > right.core[index] ? 1 : -1;
  if (!left.prerelease.length || !right.prerelease.length) return Number(!left.prerelease.length) - Number(!right.prerelease.length);
  for (let index = 0; index < Math.max(left.prerelease.length, right.prerelease.length); index++) {
    const a = left.prerelease[index], b = right.prerelease[index];
    if (a === undefined || b === undefined) return Number(a !== undefined) - Number(b !== undefined);
    if (a === b) continue;
    const numericA = /^\d+$/.test(a), numericB = /^\d+$/.test(b);
    if (numericA && numericB) return BigInt(a) > BigInt(b) ? 1 : -1;
    if (numericA !== numericB) return numericA ? -1 : 1;
    return a > b ? 1 : -1;
  }
  return 0;
}
function numericInstallerVersion(value) {
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value)) throw Error('MSI ProductVersion must be canonical major.minor.sequence');
  const parts = value.split('.').map(BigInt);
  if (parts[0] > 255n || parts[1] > 255n || parts[2] < 1n || parts[2] > 65535n) throw Error('MSI ProductVersion is outside Windows Installer limits');
  return parts;
}
export function validateVersionLedger(ledger, currentVersion) {
  parseReleaseVersion(currentVersion);
  if (ledger?.schemaVersion !== 1 || !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(ledger.upgradeCode ?? '') || !Array.isArray(ledger.versions) || ledger.versions.length === 0) throw Error('Invalid public MSI version ledger');
  const used = new Set(); let previous = null;
  for (const entry of ledger.versions) {
    const app = parseReleaseVersion(entry.appVersion), installer = numericInstallerVersion(entry.productVersion);
    if (used.has(entry.appVersion)) throw Error('MSI ledger repeats an immutable application version');
    used.add(entry.appVersion);
    if (app.core[0] !== installer[0] || app.core[1] !== installer[1]) throw Error('MSI major/minor must match application SemVer');
    if (previous) {
      if (compareReleaseVersions(entry.appVersion, previous.entry.appVersion) <= 0) throw Error('MSI ledger application versions are not strictly increasing');
      const greater = installer[0] > previous.installer[0] || installer[0] === previous.installer[0] && (installer[1] > previous.installer[1] || installer[1] === previous.installer[1] && installer[2] > previous.installer[2]);
      if (!greater) throw Error('MSI ledger ProductVersions are not strictly increasing');
    }
    previous = { entry, installer };
  }
  const mapping = ledger.versions.find(entry => entry.appVersion === currentVersion);
  if (!mapping) throw Error('Current application SemVer has no immutable MSI ledger allocation');
  return mapping;
}
export function workspaceApplicationVersion(source) {
  const section = source.split('[workspace.package]')[1]?.split(/^\[/m)[0];
  const version = section?.match(/^\s*version\s*=\s*"([^"\r\n]+)"\s*$/m)?.[1];
  parseReleaseVersion(version);
  return version;
}
export function validateReleaseConfiguration(version, workspaceVersion, configuration, ledger) {
  const mapping = validateVersionLedger(ledger, version);
  if (workspaceVersion !== version || configuration?.version !== version || configuration?.bundle?.windows?.wix?.version !== mapping.productVersion || configuration?.bundle?.windows?.wix?.upgradeCode?.toUpperCase() !== ledger.upgradeCode.toUpperCase()) throw Error('Cargo/package/configuration/MSI ledger versions or UpgradeCode differ');
  if (configuration.bundle.windows.allowDowngrades !== false) throw Error('Windows installers must explicitly reject downgrades');
  return mapping;
}
