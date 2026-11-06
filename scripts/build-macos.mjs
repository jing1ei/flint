import { spawnSync } from 'node:child_process';

// Keep local packages and CI on the same architecture-specific minimum OS.
const args = process.argv.slice(2);
const targetIndex = args.indexOf('--target');
const target = targetIndex >= 0 ? args[targetIndex + 1] : process.arch;
const arm64 = target === 'arm64' || target === 'aarch64-apple-darwin';
if (process.platform !== 'darwin') throw new Error('Build macOS packages on macOS.');
if (targetIndex >= 0 && !['aarch64-apple-darwin', 'x86_64-apple-darwin'].includes(target)) {
  throw new Error('Build separate arm64 and x86_64 macOS packages.');
}
const config = arm64 ? ['--config', 'src-tauri/tauri.arm64.conf.json'] : [];
const result = spawnSync('tauri', ['build', ...args, ...config], {
  stdio: 'inherit',
  env: { ...process.env, MACOSX_DEPLOYMENT_TARGET: arm64 ? '12.0' : '11.0' },
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
