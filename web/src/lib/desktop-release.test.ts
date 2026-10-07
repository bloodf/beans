import { expect, test } from 'bun:test'
import { parseDesktopReleases } from './desktop-release'

test('Windows downloads resolve the filename returned by GitHub uploads', () => {
  const tag = 'beans-v1.0.14'
  const prefix = `https://github.com/bloodf/beans/releases/download/${tag}/`
  const assets = ['beans-update.json', 'beans-update.json.sig', 'Lorca.Setup.1.0.14.exe']
    .map(name => ({ name, browser_download_url: prefix + name }))
  const release = { tag_name: tag, draft: false, prerelease: false, assets }
  expect(parseDesktopReleases([release])?.windows).toEqual({
    version: '1.0.14', url: prefix + 'Lorca.Setup.1.0.14.exe',
  })
  expect(parseDesktopReleases([{ ...release, assets: assets.slice(1) }])?.windows).toBeUndefined()
})
