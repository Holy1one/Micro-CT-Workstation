// Compiles and executes the unchanged candidate's in-process simulated harness.
// Outputs stay in the fixed turntable evidence area; no real hardware is accessed.
import { existsSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { config, evidenceRun, root, sourceDir, verifyCandidate } from './build-turntable-firmware.mjs';

const evidence = evidenceRun('candidate-host-harness');
try {
  const sourceIdentity = verifyCandidate();
  const exe = path.join(evidence.output, process.platform === 'win32' ? 'host_harness.exe' : 'host_harness');
  if (process.platform === 'win32') {
    const vswhere = path.join(process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)', 'Microsoft Visual Studio', 'Installer', 'vswhere.exe');
    const installation = evidence.run(vswhere, ['-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath']).trim();
    const vcvars = path.join(installation, 'VC', 'Auxiliary', 'Build', 'vcvars64.bat');
    if (!installation || !existsSync(vcvars)) throw new Error('installed MSVC host compiler unavailable');
    // A batch file establishes the compiler environment. Paths are local, validated
    // and quoted; this batch performs compilation only and cannot delete or upload.
    const unsafe = /["\r\n%&|<>^]/;
    const paths = [vcvars, path.join(sourceDir, 'host_harness.cpp'), exe, path.join(evidence.output, 'host_harness.obj')];
    if (paths.some((item) => unsafe.test(item))) throw new Error('host compiler path contains unsafe batch characters');
    const command = `@echo off\r\ncall "${paths[0]}" >nul\r\nif errorlevel 1 exit /b 1\r\ncl /nologo /std:c++17 /EHsc /W4 /O2 /utf-8 "${paths[1]}" /Fe:"${paths[2]}" /Fo:"${paths[3]}"\r\nexit /b %errorlevel%\r\n`;
    const batch = path.join(evidence.output, 'compile-host.cmd');
    writeFileSync(batch, command);
    evidence.run(process.env.ComSpec ?? 'cmd.exe', ['/d', '/c', batch], { cwd: evidence.output });
  } else {
    evidence.run(process.env.CXX ?? 'c++', ['-std=c++17', '-Wall', '-Wextra', '-O2', path.join(sourceDir, 'host_harness.cpp'), '-o', exe], { cwd: root });
  }
  const output = evidence.run(exe, []).trim();
  if (!output.startsWith('PASS actual candidate host harness:')) throw new Error('harness did not emit verified terminal evidence');
  evidence.report({ status: 'PASS', exitCode: 0, sourceIdentity, pulseScale: { microsteps: config.avr.microsteps, pulsesPerRev: config.avr.pulsesPerRev }, output });
  process.stdout.write(`${output}\n${evidence.output}\n`);
} catch (error) {
  evidence.report({ status: 'FAIL', exitCode: 1, error: String(error) });
  throw error;
}
