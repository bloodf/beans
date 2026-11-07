// Compile production sidebar controllers without the app, CLI, providers, or avatar renderer.
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
const root = resolve(import.meta.dirname, '../..');
const output = await mkdtemp(join(tmpdir(), 'beans-sidebar-chrome-'));
try {
  const rows = await readFile(join(root, 'macos/Sources/Beans/Sidebar/SidebarRows.swift'), 'utf8');
  const marker = '// MARK: - Chat row';
  if (!rows.includes(marker)) throw new Error('Sidebar row extraction boundary missing');
  await writeFile(join(output, 'SidebarRows.swift'), rows.slice(0, rows.indexOf(marker)));
  const inputs = [
    'macos/Tests/SidebarChrome.swift',
    'macos/Tests/Notifications/NativeChromeRenderingChecks.swift',
    'macos/Sources/Beans/Design/Controls.swift',
    'macos/Sources/Beans/Settings/SettingsRows.swift',
    'macos/Sources/Beans/Sidebar/SidebarViewController.swift',
    'macos/Sources/Beans/Sidebar/SettingsSidebarViewController.swift',
  ].map(path => join(root, path));
  const build = spawnSync('swiftc', [ '-parse-as-library', '-swift-version', '5', '-D', 'BEANS_CHROME_STANDALONE', '-D', 'BEANS_SIDEBAR_STANDALONE', ...inputs, join(output, 'SidebarRows.swift'), '-o', join(output, 'sidebar-chrome')], { stdio: ['ignore', 'inherit', 'inherit'] });
  if (build.error) throw build.error;
  if (build.status !== 0 || build.signal) process.exitCode = 1;
  else {
    const run = spawnSync(join(output, 'sidebar-chrome'), [], { env: { ...process.env, HOME: output, CFFIXED_USER_HOME: output, BEANS_CHROME_SHOW: process.argv.includes('--show') ? '1' : '0' }, stdio: ['ignore', 'inherit', 'inherit'] });
    if (run.error) throw run.error;
    process.exitCode = run.signal ? 1 : (run.status ?? 1);
  }
} finally { await rm(output, { recursive: true, force: true }); }
