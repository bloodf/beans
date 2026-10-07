import { i18nFor, type Language, languages, paths } from '#/i18n'
import { Nav, SITE } from './nav'
import { CallToAction, Chef, FAQ, Hero, Relay } from './sections'
import { BentoFeatures, Providers, Pricing, ComparisonLinks, MarketingFooter } from './marketing'
import { WorkExamples } from './work-examples'
import { MotionExperience } from './motion'
import displayFont from '@fontsource/manrope/files/manrope-latin-800-normal.woff2?url'
import type { LinkHTMLAttributes } from 'react'

/// The head for the landing page in one language: its title and description, plus links to
/// every language so search engines pair them up.
export function homeHead(lng: Language) {
  const t = i18nFor(lng).t
  return {
    meta: [
      { title: t('meta.title') },
      { name: 'description', content: t('meta.description') },
      { property: 'og:title', content: t('meta.title') },
      { property: 'og:description', content: t('meta.description') },
    ],
    links: [
      {
        rel: 'preload',
        href: displayFont,
        as: 'font',
        type: 'font/woff2',
        crossOrigin: 'anonymous',
      } satisfies LinkHTMLAttributes<HTMLLinkElement>,
      { rel: 'canonical', href: SITE + paths[lng] },
      ...languages.map((other) => ({
        rel: 'alternate',
        hrefLang: other,
        href: SITE + paths[other],
      })),
    ],
  }
}

export function Home() {
  return (
    <MotionExperience>
      <Nav />
      <main>
        <Hero />
        <WorkExamples />
        <BentoFeatures />
        <Providers />
        <Relay />
        <Pricing />
        <ComparisonLinks />
        <Chef />
        <FAQ />
        <CallToAction />
      </main>
      <MarketingFooter />
    </MotionExperience>
  )
}
