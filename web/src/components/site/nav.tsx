import { Link, useLocation, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '#/components/ui/button'
import {
  currentLanguage,
  isLanguage,
  languages,
  names,
  pathIn,
  paths,
  localizedPath,
} from '#/i18n'

export const SITE = 'https://usebeans.app'

const links = [
  { id: 'turns', label: 'nav.turns' },
  { id: 'relay', label: 'nav.relay' },
  { id: 'tools', label: 'nav.tools' },
  { id: 'costs', label: 'nav.costs' },
] as const

/// The docs in the page's language: `/docs` and `/zh/docs`, or one of their pages: `/docs/cli`.
export function docsPath(lng: string, page?: string) {
  const docs = lng === 'zh' ? '/zh/docs' : '/docs'
  return page ? `${docs}/${page}` : docs
}

/// The download page in the page's language: `/download` and `/zh/download`. It links the Mac
/// app's disk image, the Windows installer, the Linux install script and Debian packages, the
/// iPhone beta, and the CLI installer.
export function downloadPath(lng: string) {
  return localizedPath(lng, '/download')
}

/// A section of the landing page, from any page: `/#faq` and `/zh#faq`. It is the current page
/// only at its own section. Without `resetScroll`, a second click on the section already in the
/// address bar restores the scroll position the click was made at instead of scrolling to it.
export function SectionLink({
  id,
  ...props
}: { id: string } & Omit<React.ComponentProps<'a'>, 'href'>) {
  const { i18n } = useTranslation()
  return (
    <Link
      to={paths[currentLanguage(i18n.language)]}
      hash={id}
      activeOptions={{ includeHash: true }}
      resetScroll={false}
      {...props}
    />
  )
}

export function LanguageLink({ className }: { className?: string }) {
  const { t, i18n } = useTranslation()
  const location = useLocation()
  const navigate = useNavigate()
  return (
    <select
      className={`site-language-select ${className ?? ''}`}
      aria-label={t('nav.language')}
      value={currentLanguage(i18n.language)}
      onChange={(event) => {
        const value = event.currentTarget.value
        if (!isLanguage(value)) return
        void navigate({
          to: pathIn(value, location.pathname) + location.searchStr,
          hash: location.hash,
        })
      }}
    >
      {languages.map((language) => (
        <option key={language} value={language} lang={language}>
          {names[language]}
        </option>
      ))}
    </select>
  )
}

/// The site header stays visible as the page scrolls.
export function Nav() {
  const { t, i18n } = useTranslation()
  return (
    <header className="site-nav sticky top-0 z-40 border-b bg-background/95 px-4 backdrop-blur-xl">
      <div className="mx-auto flex h-18 max-w-[1400px] items-center justify-between gap-4">
        <SectionLink
          id="top"
          className="flex items-center gap-2 font-semibold tracking-tight"
        >
          <img
            src="/brand/beans-logo.svg"
            alt="Beans"
            width={2161}
            height={728}
            className="w-36 dark:hidden"
          />
          <img
            src="/brand/beans-logo-light.svg"
            alt="Beans"
            width={2161}
            height={728}
            className="hidden w-36 dark:block"
          />
        </SectionLink>
        <nav className="hidden items-center gap-6 text-sm text-muted-foreground xl:flex">
          {links.map((link) => (
            <SectionLink
              key={link.id}
              id={link.id}
              className="transition-colors hover:text-foreground"
            >
              {t(link.label)}
            </SectionLink>
          ))}
          <Link
            to={localizedPath(i18n.language, '/compare')}
            className="transition-colors hover:text-foreground"
          >
            {t('nav.compare')}
          </Link>
          <Link
            viewTransition
            to={docsPath(i18n.language)}
            className="transition-colors hover:text-foreground"
          >
            {t('nav.docs')}
          </Link>
        </nav>
        <div className="flex items-center gap-3">
          <LanguageLink className="text-sm text-muted-foreground transition-colors hover:text-foreground" />
          <Button asChild size="sm" className="rounded-full px-4">
            <Link viewTransition to={downloadPath(i18n.language)}>
              {t('nav.download')}
            </Link>
          </Button>
        </div>
      </div>
    </header>
  )
}
