// Rebuilds the hash-pinned Nano candidate using the installed pinned AVR core.
// Inputs are source and compiler packages; outputs stay in tmp/tests/turntable.
// No discovery, serial port, uploader, flash read, or hardware operation exists here.
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const sourceDir = path.join(root, 'firmware', 'rts9060_nano');
export const config = JSON.parse(readFileSync(path.join(sourceDir, 'build-config.json'), 'utf8'));
export const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
export function verifyCandidate() {
  const bytes = readFileSync(path.join(sourceDir, config.candidate.source));
  if (bytes.length !== config.candidate.bytes || sha256(bytes) !== config.candidate.sha256) {
    throw new Error('candidate source identity mismatch; do not rebuild a silently changed candidate');
  }
  return { bytes: bytes.length, sha256: sha256(bytes) };
}
export function evidenceRun(topic) {
  const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
  const output = path.join(root, 'tmp', 'tests', 'turntable', `${stamp}-${topic}`);
  mkdirSync(output, { recursive: true });
  const commands = [];
  let stdout = ''; let stderr = '';
  function run(executable, args, options = {}) {
    commands.push(JSON.stringify({ executable, args, cwd: options.cwd ?? root }));
    const result = spawnSync(executable, args, { cwd: root, encoding: 'utf8', windowsHide: true, ...options });
    stdout += result.stdout ?? ''; stderr += result.stderr ?? '';
    writeFileSync(path.join(output, 'command.txt'), commands.join('\n') + '\n');
    writeFileSync(path.join(output, 'stdout.log'), stdout);
    writeFileSync(path.join(output, 'stderr.log'), stderr);
    if (result.error || result.status !== 0) throw new Error(`${path.basename(executable)} failed: ${result.error ?? result.stderr ?? result.stdout}`);
    return result.stdout;
  }
  const report = (details) => writeFileSync(path.join(output, 'result.json'), JSON.stringify({
    time: new Date().toISOString(), onlyOffline: true, hardwareCertified: false,
    blocked: ['current flash byte identity', 'physical direction and load-side accuracy', 'AVR ISR race timing'],
    ...details,
  }, null, 2) + '\n');
  return { output, run, report };
}

function build() {
  const evidence = evidenceRun('candidate-avr-build');
  try {
    const sourceIdentity = verifyCandidate();
    const arduino15 = process.env.ARDUINO15 ?? path.join(process.env.LOCALAPPDATA, 'Arduino15');
    const packageRoot = path.join(arduino15, 'packages', 'arduino');
    const corePackage = path.join(packageRoot, 'hardware', 'avr', config.avr.coreVersion);
    const core = path.join(corePackage, 'cores', 'arduino');
    const variant = path.join(corePackage, 'variants', config.avr.variant);
    const compiler = path.join(packageRoot, 'tools', 'avr-gcc', config.avr.compilerVersion, 'bin');
    const tool = (name) => path.join(compiler, `${name}${process.platform === 'win32' ? '.exe' : ''}`);
    const compilerVersion = evidence.run(tool('avr-g++'), ['--version']).trim();
    if (!compilerVersion.includes('7.3.0')) throw new Error('unexpected AVR GCC version');
    if (!readFileSync(path.join(corePackage, 'platform.txt'), 'utf8').includes(`version=${config.avr.coreVersion}`)) throw new Error('unexpected AVR core version');
    const target = [`-mmcu=${config.avr.mcu}`, `-DF_CPU=${config.avr.cpuHz}L`, `-DARDUINO=${config.avr.arduinoVersion}`, '-DARDUINO_AVR_NANO', '-DARDUINO_ARCH_AVR', `-I${core}`, `-I${variant}`];
    const common = ['-c', '-g', '-Os', '-w', '-ffunction-sections', '-fdata-sections', '-MMD', '-flto'];
    const cpp = ['-std=gnu++11', '-fpermissive', '-fno-exceptions', '-fno-threadsafe-statics', '-Wno-error=narrowing'];
    const source = path.join(sourceDir, config.candidate.source);
    const sketchObject = path.join(evidence.output, 'rts9060_nano.ino.o');
    evidence.run(tool('avr-g++'), [...common, ...cpp, ...target, '-x', 'c++', source, '-o', sketchObject]);
    const objects = [];
    for (const name of readdirSync(core).filter((name) => /\.(c|cpp|S)$/.test(name)).sort()) {
      const ext = path.extname(name);
      const object = path.join(evidence.output, `${name}.o`);
      const flags = ext === '.cpp' ? [...common, ...cpp] : ext === '.c' ? [...common, '-std=gnu11', '-fno-fat-lto-objects'] : ['-c', '-g', '-x', 'assembler-with-cpp', '-flto', '-MMD'];
      evidence.run(tool(ext === '.cpp' ? 'avr-g++' : 'avr-gcc'), [...flags, ...target, path.join(core, name), '-o', object]);
      objects.push(object);
    }
    const archive = path.join(evidence.output, 'core.a');
    evidence.run(tool('avr-gcc-ar'), ['rcs', archive, ...objects]);
    const elf = path.join(evidence.output, 'rts9060_nano.elf');
    evidence.run(tool('avr-gcc'), ['-w', '-Os', '-g', '-flto', '-fuse-linker-plugin', '-Wl,--gc-sections', `-mmcu=${config.avr.mcu}`, '-o', elf, sketchObject, archive, `-L${evidence.output}`, '-lm']);
    const hex = path.join(evidence.output, 'rts9060_nano.hex');
    const binary = path.join(evidence.output, 'rts9060_nano.bin');
    evidence.run(tool('avr-objcopy'), ['-O', 'ihex', '-R', '.eeprom', elf, hex]);
    evidence.run(tool('avr-objcopy'), ['-O', 'binary', '-R', '.eeprom', elf, binary]);
    const sizeText = evidence.run(tool('avr-size'), ['-A', elf]);
    const sections = Object.fromEntries([...sizeText.matchAll(/^(\.\S+)\s+(\d+)\s+/gm)].map((match) => [match[1], Number(match[2])]));
    const flashBytes = (sections['.text'] ?? 0) + (sections['.data'] ?? 0);
    const ramBytes = (sections['.data'] ?? 0) + (sections['.bss'] ?? 0) + (sections['.noinit'] ?? 0);
    if (flashBytes > config.avr.flashMaximumBytes || ramBytes > config.avr.ramMaximumBytes) throw new Error('candidate exceeds fixed Nano memory budget');
    const artifacts = [elf, hex, binary].map((file) => { const bytes = readFileSync(file); return { name: path.basename(file), bytes: bytes.length, sha256: sha256(bytes) }; });
    evidence.report({ status: 'PASS', exitCode: 0, sourceIdentity, config: config.avr, compilerVersion, flashBytes, ramBytes, artifacts });
    process.stdout.write(`PASS AVR source rebuild: flash=${flashBytes} RAM=${ramBytes}\n${evidence.output}\n`);
  } catch (error) {
    evidence.report({ status: 'FAIL', exitCode: 1, error: String(error) });
    throw error;
  }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) build();
