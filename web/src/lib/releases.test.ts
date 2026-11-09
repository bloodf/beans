import { afterEach, expect, test, vi } from 'vitest'
import { fetchDesktopRelease, parseDesktopReleases } from './releases'

function release(version: string, names: string[]) {
  const tag_name = `beans-v${version}`
  const prefix = `https://github.com/bloodf/beans/releases/download/${tag_name}/`
  return { tag_name, draft: false, prerelease: false,
    assets: names.map(name => ({ name, browser_download_url: prefix + name })) }
}
const ready = ['beans-update.json', 'beans-update.json.sig']
afterEach(() => vi.unstubAllGlobals())

test('selects platform versions independently across server-only and incomplete releases', () => {
  const clients = parseDesktopReleases([
    release('1.0.20', ready),
    release('1.0.19', [...ready, 'Beans-1.0.19.dmg']),
    release('1.0.18', [...ready, 'Beans-1.0.18.apk', 'Beans-1.0.18.ipa']),
    release('1.0.17', [...ready, 'Beans-1.0.17.dmg', 'Beans-1.0.17.zip', 'appcast.xml']),
    release('1.0.16', [...ready, 'install-linux-amd64.sh', 'beans_1.0.16_amd64.deb', 'beans_1.0.16_arm64.deb']),
  ])
  expect(clients?.mac).toEqual({ version: '1.0.17',
    url: 'https://github.com/bloodf/beans/releases/download/beans-v1.0.17/Beans-1.0.17.dmg',
    appcast: 'https://github.com/bloodf/beans/releases/download/beans-v1.0.17/appcast.xml' })
  expect(clients?.android?.version).toBe('1.0.18')
  expect(clients?.ios?.version).toBe('1.0.18')
  expect(clients?.linux?.version).toBe('1.0.16')
  expect(clients?.windows).toBeNull()
})

test('requires stable finalized Beans assets and retains numeric ordering', () => {
  const good = release('1.0.9', [...ready, 'Beans-1.0.9.apk'])
  const newer = release('1.0.10', [...ready, 'Beans-1.0.10.apk'])
  expect(parseDesktopReleases([good, newer])?.android?.version).toBe('1.0.10')
  for (const invalid of [
    { ...newer, draft: true }, { ...newer, prerelease: true },
    { ...newer, tag_name: 'mobile-v1.0.10' },
    release('1.0.10', ['beans-update.json', 'Beans-1.0.10.apk']),
  ]) expect(parseDesktopReleases([good, invalid])?.android?.version).toBe('1.0.9')
})

test('rejects malformed and foreign download URLs', () => {
  expect(() => parseDesktopReleases({})).toThrow('malformed')
  const foreign = release('1.0.1', ready)
  foreign.assets[0].browser_download_url = 'https://github.com/egoist/lorca/releases/download/beans-v1.0.1/beans-update.json'
  expect(() => parseDesktopReleases([foreign])).toThrow('malformed')
})

test('confirmed absence is valid in production but API failure is fatal', async () => {
  const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => [release('1.0.20', ready)] })
  vi.stubGlobal('fetch', fetcher)
  expect(await fetchDesktopRelease({ required: true })).toBeNull()
  expect(fetcher.mock.calls[0][0]).toBe('https://api.github.com/repos/bloodf/beans/releases?per_page=100')
  fetcher.mockResolvedValue({ ok: false, status: 403 })
  await expect(fetchDesktopRelease({ required: true })).rejects.toThrow('HTTP 403')
})
