// Compile production sidebar controllers without the app, CLI, providers, or avatar renderer.
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
const root = resolve(import.meta.dir, '../..');
const output = await mkdtemp(join(tmpdir(), 'beans-sidebar-chrome-'));
try {
  const rows = await Bun.file(join(root, 'macos/Sources/Lorca/Sidebar/SidebarRows.swift')).text();
  const marker = '// MARK: - Chat row';
  if (!rows.includes(marker)) throw new Error('Sidebar row extraction boundary missing');
  await Bun.write(join(output, 'SidebarRows.swift'), rows.slice(0, rows.indexOf(marker)));
  const inputs = [
    'macos/Tests/SidebarChrome.swift',
    'macos/Tests/Notifications/NativeChromeRenderingChecks.swift',
    'macos/Sources/Lorca/Design/Controls.swift',
    'macos/Sources/Lorca/Settings/SettingsRows.swift',
    'macos/Sources/Lorca/Sidebar/SidebarViewController.swift',
    'macos/Sources/Lorca/Sidebar/SettingsSidebarViewController.swift',
  ].map(path => join(root, path));
  const build = Bun.spawn(['swiftc', '-parse-as-library', '-swift-version', '5', '-D', 'BEANS_CHROME_STANDALONE', '-D', 'BEANS_SIDEBAR_STANDALONE', ...inputs, join(output, 'SidebarRows.swift'), '-o', join(output, 'sidebar-chrome')], { stdout: 'inherit', stderr: 'inherit' });
  if (await build.exited !== 0) process.exitCode = 1;
  else {
    const run = Bun.spawn([join(output, 'sidebar-chrome')], { env: { ...process.env, HOME: output, CFFIXED_USER_HOME: output, BEANS_CHROME_SHOW: process.argv.includes('--show') ? '1' : '0' }, stdout: 'inherit', stderr: 'inherit' });
    process.exitCode = await run.exited;
  }
} finally { await rm(output, { recursive: true, force: true }); }
