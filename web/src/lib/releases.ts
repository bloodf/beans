/// <reference types="node" />

/// Client downloads come from finalized stable Beans GitHub releases.
/// Server-only readiness does not imply that any client artifact is available.
const RELEASES = 'https://api.github.com/repos/bloodf/beans/releases?per_page=100'
const TAG = 'beans-v'

type ClientArtifact = { version: string; url: string }

export type DesktopRelease = {
  mac: (ClientArtifact & { appcast: string }) | null
  windows: ClientArtifact | null
  linux: {
    version: string
    /// Installs the app in ~/.local, where it updates itself.
    installScript: string
    /// Debian packages install in /opt and update with the next package.
    deb: { amd64: string; arm64: string }
  } | null
  android: ClientArtifact | null
  ios: ClientArtifact | null
}

type GitHubRelease = {
  tag_name: string
  draft: boolean
  prerelease: boolean
  assets: { name: string; browser_download_url: string }[]
}

const numeric = new Intl.Collator('en', { numeric: true })

function isGitHubRelease(value: unknown): value is GitHubRelease {
  if (typeof value !== 'object' || value === null
    || !('tag_name' in value) || typeof value.tag_name !== 'string' || !value.tag_name
    || !('draft' in value) || typeof value.draft !== 'boolean'
    || !('prerelease' in value) || typeof value.prerelease !== 'boolean'
    || !('assets' in value) || !Array.isArray(value.assets)) return false
  const prefix = `https://github.com/bloodf/beans/releases/download/${encodeURIComponent(value.tag_name)}/`
  return value.assets.every((asset: unknown) =>
      typeof asset === 'object' && asset !== null
      && 'name' in asset && typeof asset.name === 'string' && asset.name.length > 0
      && 'browser_download_url' in asset && typeof asset.browser_download_url === 'string'
      && asset.browser_download_url.startsWith(prefix)
      && asset.browser_download_url.length > prefix.length)
}

/// Select each platform's newest ready assets independently. A newer server-only release does
/// not hide an older client release. A valid API response with no eligible clients is unavailable.
export function parseDesktopReleases(releases: unknown): DesktopRelease | null {
  if (!Array.isArray(releases) || !releases.every(isGitHubRelease))
    throw new Error('malformed GitHub releases response')
  const ready = releases
    .filter((release) => /^beans-v\d+\.\d+\.\d+$/.test(release.tag_name) && !release.draft && !release.prerelease
      && release.assets.some(({ name }) => name === 'beans-update.json')
      && release.assets.some(({ name }) => name === 'beans-update.json.sig'))
    .sort((a, b) => numeric.compare(b.tag_name, a.tag_name))
  const clients: DesktopRelease = { mac: null, windows: null, linux: null, android: null, ios: null }
  for (const release of ready) {
    const version = release.tag_name.slice(TAG.length)
    const file = (name: string) => release.assets.find((asset) => asset.name === name)?.browser_download_url
    const dmg = file(`Beans-${version}.dmg`)
    const appcast = file('appcast.xml')
    if (!clients.mac && dmg && appcast && file(`Beans-${version}.zip`))
      clients.mac = { version, url: dmg, appcast }
    const windows = file(`Beans Setup ${version}.exe`) ?? file(`Beans.Setup.${version}.exe`)
    if (!clients.windows && windows) clients.windows = { version, url: windows }
    const installScript = file('install-linux-amd64.sh')
    const amd64 = file(`beans_${version}_amd64.deb`)
    const arm64 = file(`beans_${version}_arm64.deb`)
    if (!clients.linux && installScript && amd64 && arm64)
      clients.linux = { version, installScript, deb: { amd64, arm64 } }
    const android = file(`Beans-${version}.apk`)
    if (!clients.android && android) clients.android = { version, url: android }
    const ios = file(`Beans-${version}.ipa`)
    if (!clients.ios && ios) clients.ios = { version, url: ios }
  }
  return Object.values(clients).some(Boolean) ? clients : null
}

/// Reads client availability for the build, with `GITHUB_TOKEN` or `GH_TOKEN` when set.
/// API, authentication, network and malformed-response failures stop production builds;
/// the dev server warns and disables downloads. Confirmed client absence returns null in either mode.
export async function fetchDesktopRelease({ required }: { required: boolean }): Promise<DesktopRelease | null> {
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN
  try {
    const response = await fetch(RELEASES, {
      headers: {
        accept: 'application/vnd.github+json',
        'user-agent': 'beans',
        ...(token && { authorization: `Bearer ${token}` }),
      },
      signal: AbortSignal.timeout(10_000),
    })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    return parseDesktopReleases(await response.json())
  } catch (error) {
    const message = `reading ${RELEASES}: ${error instanceof Error ? error.message : error}`
    if (required) throw new Error(message)
    console.warn(message)
    return null
  }
}
