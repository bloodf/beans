import { Link } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  Check,
  Computer,
  Copy,
  Laptop,
  type LucideIcon,
  Monitor,
  Smartphone,
  SquareTerminal,
  TabletSmartphone,
} from 'lucide-react'
import { Fragment, useState } from 'react'
import { Trans, useTranslation } from 'react-i18next'

import { Button } from '#/components/ui/button'
import { i18nFor, type Language, languages } from '#/i18n'
import type { DesktopRelease } from '#/lib/desktop-release'
import { cn } from '#/lib/utils'
import { Logo } from './logo'
import { Nav, SITE, docsPath, downloadPath } from './nav'
import { Footer } from './sections'

/// Bots on Windows run their commands in its bash.
const GIT_FOR_WINDOWS = 'https://git-scm.com/downloads/win'
/// The CLI's install commands, one per shell.
const INSTALL = [
  { shell: 'download.cli.unix', prompt: '$', command: 'curl -fsSL https://raw.githubusercontent.com/bloodf/beans/main/web/public/install-cli.sh | sh' },
  { shell: 'download.cli.windows', prompt: 'PS>', command: 'irm https://raw.githubusercontent.com/bloodf/beans/main/web/public/install-cli.ps1 | iex' },
] as const

export type MacRelease = {
  version: string
  /// The notarized disk image.
  url: string
  minimumSystemVersion: string | null
  appleSilicon: boolean
}

/// Per-platform ready client assets, which the build reads (`define` in vite.config.ts).
declare const __DESKTOP_RELEASE__: DesktopRelease | null

const numeric = new Intl.Collator('en', { numeric: true })

/// Version and system requirements from the newest item in a Sparkle appcast.
/// The download URL comes from the selected release's actual disk-image asset.
export function parseAppcast(xml: string): Omit<MacRelease, 'url'> | null {
  const items = [...xml.matchAll(/<item\b[^>]*>([\s\S]*?)<\/item>/g)].flatMap(([, item]) => {
    // Delta updates carry enclosures of their own.
    const body = item.replace(/<sparkle:deltas>[\s\S]*?<\/sparkle:deltas>/g, '')
    const field = (name: string) => body.match(new RegExp(`<${name}>([^<]*)</${name}>`))?.[1].trim()
    const archive = body.match(/<enclosure\b[^>]*\burl="([^"]+)"/)?.[1]
    const build = field('sparkle:version')
    if (!archive || !build) return []
    const release: Omit<MacRelease, 'url'> = {
      version: field('sparkle:shortVersionString') ?? build,
      minimumSystemVersion: field('sparkle:minimumSystemVersion') ?? null,
      appleSilicon: field('sparkle:hardwareRequirements')?.includes('arm64') ?? false,
    }
    return [{ build, release }]
  })
  return items.sort((a, b) => numeric.compare(b.build, a.build))[0]?.release ?? null
}

/// The loader of the download page, in either language. Read only an appcast from a release
/// that actually contains Mac assets; a newer server-only release has no effect on this selection.
/// An unreadable feed leaves the Mac button disabled and the rest of the page working.
export const latestMacRelease = createServerFn({ method: 'GET' }).handler(async () => {
  const selected = __DESKTOP_RELEASE__?.mac
  if (!selected) return null
  try {
    const response = await fetch(selected.appcast, { signal: AbortSignal.timeout(5000) })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const release = parseAppcast(await response.text())
    if (!release) throw new Error('no release in the feed')
    if (release.version !== selected.version) throw new Error('appcast version does not match the selected release')
    return { ...release, url: selected.url }
  } catch (error) {
    console.error(`reading ${selected.appcast}:`, error)
    return null
  }
})

/// The head for the download page in one language, paired with the other languages.
export function downloadHead(lng: Language) {
  const t = i18nFor(lng).t
  return {
    meta: [
      { title: t('download.title') },
      { name: 'description', content: t('download.description') },
      { property: 'og:title', content: t('download.title') },
      { property: 'og:description', content: t('download.description') },
    ],
    links: [
      { rel: 'canonical', href: SITE + downloadPath(lng) },
      ...languages.map((other) => ({ rel: 'alternate', hrefLang: other, href: SITE + downloadPath(other) })),
    ],
  }
}

const action = 'h-12 rounded-full px-7 text-base'
const link = 'underline underline-offset-4 hover:text-foreground'

export function Download({ release: mac }: { release: MacRelease | null }) {
  const { t, i18n } = useTranslation()
  const desktop = __DESKTOP_RELEASE__
  const windows = desktop?.windows
  const linux = desktop?.linux
  const ios = desktop?.ios
  const android = desktop?.android
  return (
    <div className="flex min-h-svh flex-col">
      <Nav />
      <main className="flex-1">
        <section className="mx-auto max-w-4xl px-5 pt-20 text-center sm:pt-28">
          <Logo className="mx-auto size-20 drop-shadow-xl" />
          <h1 className="display mt-6 text-[3.4rem] text-balance sm:text-[4.6rem]">{t('download.title')}</h1>
        </section>
        <section className="mx-auto grid max-w-4xl gap-5 px-5 pt-12 sm:pt-16 md:grid-cols-2">
          <Platform
            icon={Laptop}
            title={t('download.mac.title')}
            body={t('download.mac.body')}
            note={
              mac ? (
                <Notes
                  items={[
                    t('download.version', { version: mac.version }),
                    mac.minimumSystemVersion &&
                      t('download.mac.system', { version: mac.minimumSystemVersion.replace(/(\.0)+$/, '') }),
                    mac.appleSilicon && t('download.mac.appleSilicon'),
                  ]}
                />
              ) : (
                t(desktop?.mac ? 'download.loadFailed' : 'download.unavailable')
              )
            }
          >
            <DownloadButton href={mac?.url}>{t('download.mac.action')}</DownloadButton>
          </Platform>
          <Platform
            icon={Monitor}
            title={t('download.windows.title')}
            body={
              <Trans i18nKey="download.windows.body" components={{ git: <a href={GIT_FOR_WINDOWS} className={link} /> }} />
            }
            note={
              windows ? (
                <Notes items={[t('download.version', { version: windows.version }), t('download.windows.system')]} />
              ) : (
                t('download.unavailable')
              )
            }
          >
            <DownloadButton href={windows?.url}>{t('download.windows.action')}</DownloadButton>
          </Platform>
          {/* The whole row, for the install command's URL. */}
          <Platform
            className="md:col-span-2"
            icon={Computer}
            title={t('download.linux.title')}
            body={t('download.linux.body')}
            note={
              linux ? (
                <>
                  <Trans
                    i18nKey="download.linux.deb"
                    components={{
                      amd64: <a href={linux.deb.amd64} className={link} />,
                      arm64: <a href={linux.deb.arm64} className={link} />,
                    }}
                  />
                  <br />
                  <Notes items={[t('download.version', { version: linux.version }), t('download.linux.system')]} />
                </>
              ) : (
                t('download.unavailable')
              )
            }
          >
            {linux ? (
              <InstallCommand prompt="$" command={`curl -fsSL ${linux.installScript} | sh`} />
            ) : (
              <DownloadButton href={undefined}>{t('download.linux.action')}</DownloadButton>
            )}
          </Platform>
          <Platform
            icon={TabletSmartphone}
            title={t('download.ios.title')}
            body={t('download.ios.body')}
            note={ios ? (
              <>
                <Notes items={[t('download.version', { version: ios.version })]} />
                <br />
                {t('download.ios.note')}
              </>
            ) : t('download.unavailable')}
          >
            <DownloadButton href={ios?.url}>{t('download.ios.action')}</DownloadButton>
          </Platform>
          <Platform
            icon={Smartphone}
            title={t('download.android.title')}
            body={t('download.android.body')}
            note={android ? t('download.version', { version: android.version }) : t('download.unavailable')}
          >
            <DownloadButton href={android?.url}>{t('download.android.action')}</DownloadButton>
          </Platform>
        </section>
        <section className="mx-auto max-w-4xl px-5 pt-5 pb-20">
          <Platform
            icon={SquareTerminal}
            title={t('download.cli.title')}
            body={t('download.cli.body')}
            note={
              <Trans
                i18nKey="download.cli.note"
                components={{
                  docs: <Link to={docsPath(i18n.language, 'cli')} className={link} />,
                }}
              />
            }
          >
            <div className="space-y-5">
              {INSTALL.map(({ shell, prompt, command }) => (
                <div key={command}>
                  <p className="mb-2 text-sm text-muted-foreground">{t(shell)}</p>
                  <InstallCommand prompt={prompt} command={command} />
                </div>
              ))}
            </div>
          </Platform>
        </section>
      </main>
      <Footer />
    </div>
  )
}

/// A platform's download, disabled when no eligible artifact is available.
function DownloadButton({ href, children }: { href: string | undefined; children: React.ReactNode }) {
  return href ? (
    <Button asChild size="lg" className={action}>
      <a href={href}>{children}</a>
    </Button>
  ) : (
    <Button size="lg" disabled className={action}>
      {children}
    </Button>
  )
}

/// Version and system requirements. A narrow card wraps the line between items, never inside one.
function Notes({ items }: { items: (string | false | null)[] }) {
  return items
    .filter((item): item is string => Boolean(item))
    .map((item, i) => (
      <Fragment key={item}>
        {i > 0 && ' · '}
        <span className="whitespace-nowrap">{item}</span>
      </Fragment>
    ))
}

/// An install command. A click anywhere on it copies it; the text stays selectable.
function InstallCommand({ prompt, command }: { prompt: string; command: string }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  const copy = () =>
    navigator.clipboard.writeText(command).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  const Icon = copied ? Check : Copy
  return (
    <div
      onClick={copy}
      className="flex cursor-pointer items-center gap-3 rounded-2xl border bg-foreground/[0.03] px-4 py-3.5 font-mono text-sm transition-colors hover:bg-foreground/5"
    >
      <span className="text-muted-foreground/60 select-none" aria-hidden="true">
        {prompt}
      </span>
      {/* One line, like a terminal: a narrow screen scrolls it rather than wrapping the URL. */}
      <code className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap">{command}</code>
      {/* For the keyboard: its click bubbles to the block. */}
      <button
        type="button"
        aria-label={t(copied ? 'download.cli.copied' : 'download.cli.copy')}
        title={t(copied ? 'download.cli.copied' : 'download.cli.copy')}
        className="-m-1 rounded-md p-1 text-muted-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
      >
        <Icon className="size-4" />
      </button>
    </div>
  )
}


function Platform({
  icon: Icon,
  title,
  body,
  note,
  className,
  children,
}: {
  icon: LucideIcon
  title: string
  body: React.ReactNode
  /// A line under the button: version and requirements.
  note?: React.ReactNode
  className?: string
  /// The button, the install command, or the coming-soon mark.
  children: React.ReactNode
}) {
  return (
    <div className={cn('panel flex flex-col px-7 py-9 sm:px-10 sm:py-10', className)}>
      <Icon className="size-6 text-violet" strokeWidth={1.75} />
      <h2 className="mt-6 text-2xl font-semibold tracking-tight">{title}</h2>
      <p className="mt-3 max-w-2xl leading-relaxed text-muted-foreground">{body}</p>
      <div className="mt-auto pt-8">
        {children}
        {note && <p className="mt-4 text-sm text-muted-foreground/80">{note}</p>}
      </div>
    </div>
  )
}
