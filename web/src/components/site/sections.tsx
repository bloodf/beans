import { Link } from '@tanstack/react-router'
import { Brain, FilePen, Globe, Plug } from 'lucide-react'
import { Trans, useTranslation } from 'react-i18next'

import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from '#/components/ui/accordion'
import { Button } from '#/components/ui/button'
import { RelayDiagram } from './diagram'
import { Logo } from './logo'
import { LanguageLink, SectionLink, docsPath, downloadPath } from './nav'

export function Hero() {
  const { t, i18n } = useTranslation()
  return (
    <section id="top" className="beans-hero mx-auto grid max-w-6xl items-center gap-12 px-5 pt-16 pb-12 sm:pt-24 lg:grid-cols-[1.2fr_0.8fr]">
      <div>
      <p className="mb-5 inline-flex items-center gap-2 rounded-full border bg-foreground/5 px-3 py-1 text-xs font-medium text-foreground/80">
        <span className="size-1.5 rounded-full bg-[#F26744]" />
        {t('hero.badge')}
      </p>
      <h1 className="display text-[3.8rem] text-balance sm:text-[5.4rem] lg:text-[6.4rem]">
        <Trans i18nKey="hero.title" components={{ accent: <span className="brush" /> }} />
      </h1>
      <p className="mt-7 max-w-lg text-lg text-pretty text-muted-foreground sm:text-xl">
        {t('hero.body')}
      </p>
      <div className="mt-9 flex flex-col gap-3 sm:flex-row">
        <Button asChild size="lg" className="h-12 rounded-full px-7 text-base">
          <Link to={downloadPath(i18n.language)}>{t('nav.download')}</Link>
        </Button>
        <Button asChild size="lg" variant="outline" className="h-12 rounded-full bg-transparent px-6 text-base shadow-none hover:bg-foreground/5 dark:bg-transparent dark:hover:bg-foreground/5">
          <SectionLink id="turns">{t('hero.how')}</SectionLink>
        </Button>
      </div>
      </div>
      <div className="beans-hero-art" aria-hidden="true">
        <img src="/brand/beans-mark.svg" alt="" width={1254} height={1254} className="w-full" />
      </div>
    </section>
  )
}

function Stage({
  id,
  title,
  body,
  children,
  bare = false,
}: {
  id: string
  title: React.ReactNode
  body: string
  children: React.ReactNode
  /// No backdrop: the children sit on the panel's own surface, edge to edge.
  bare?: boolean
}) {
  return (
    <section id={id} className="mx-auto max-w-6xl scroll-mt-8 px-5 py-10">
      <div className="panel overflow-hidden">
        <div className="px-6 pt-10 pb-8 sm:px-12 sm:pt-14">
          <h2 className="display mt-3 max-w-3xl text-4xl sm:text-5xl">{title}</h2>
          <p className="mt-5 max-w-3xl text-lg leading-relaxed text-muted-foreground">{body}</p>
        </div>
        {bare ? (
          children
        ) : (
          <div className="beans-stage relative">
            <div className="relative px-4 py-10 sm:px-12 sm:py-14">{children}</div>
          </div>
        )}
      </div>
    </section>
  )
}

export function Turns() {
  const { t } = useTranslation()
  return (
    <Stage
      id="turns"
      title={t('turns.title')}
      body={t('turns.body')}
    >
      <img
        src="/screens/group.png"
        width={1568}
        height={993}
        alt={t('turns.alt')}
        className="window-frame mx-auto w-full max-w-5xl"
        loading="lazy"
        decoding="async"
      />
    </Stage>
  )
}

export function Relay() {
  const { t } = useTranslation()
  return (
    <Stage
      id="relay"
      title={t('relay.title')}
      body={t('relay.body')}
    >
      <div className="mx-auto max-w-4xl">
        <RelayDiagram />
      </div>
    </Stage>
  )
}

/// What a bot can do once it has a folder, in the words a first-time visitor uses.
const toolKinds = [
  { key: 'files', icon: FilePen },
  { key: 'web', icon: Globe },
  { key: 'memory', icon: Brain },
  { key: 'plugins', icon: Plug },
] as const

export function Tools() {
  const { t } = useTranslation()
  return (
    <Stage
      id="tools"
      bare
      title={t('tools.title')}
      body={t('tools.body')}
    >
      {/* Hairlines are the grid's own background showing through one-pixel gaps. */}
      <ul className="grid gap-px bg-border pt-px sm:grid-cols-2 lg:grid-cols-4">
        {toolKinds.map(({ key, icon: Icon }) => (
          <li key={key} className="bg-card px-6 py-8 sm:px-8">
            <Icon className="size-5 text-violet" strokeWidth={1.75} />
            <p className="mt-5 font-semibold">{t(`tools.kinds.${key}.title`)}</p>
            <p className="mt-2 leading-relaxed text-muted-foreground">{t(`tools.kinds.${key}.body`)}</p>
          </li>
        ))}
      </ul>
    </Stage>
  )
}

export function Chef() {
  const { t } = useTranslation()
  return (
    <section id="start" className="mx-auto max-w-6xl scroll-mt-8 px-5 py-10">
      <div className="panel grid gap-10 px-6 py-12 sm:px-12 lg:grid-cols-[1fr_1fr] lg:items-center">
        <div>
          <p className="text-xs font-semibold tracking-[0.18em] text-muted-foreground/80 uppercase">{t('chef.eyebrow')}</p>
          <h2 className="display mt-3 text-4xl sm:text-5xl">{t('chef.title')}</h2>
          <p className="mt-5 text-lg leading-relaxed text-muted-foreground">
            {t('chef.body')}
          </p>
        </div>
        <ol className="space-y-5">
          {t('chef.steps', { returnObjects: true }).map(({ title, body }, i) => (
            <li key={title} className="flex gap-4 rounded-2xl border bg-foreground/[0.03] p-4">
              <span className="font-mono text-sm text-muted-foreground/80">0{i + 1}</span>
              <div>
                <p className="font-semibold">{title}</p>
                <p className="mt-1 text-sm text-muted-foreground">{body}</p>
              </div>
            </li>
          ))}
        </ol>
      </div>
    </section>
  )
}

export function FAQ() {
  const { t } = useTranslation()
  return (
    <section id="faq" className="mx-auto max-w-3xl scroll-mt-8 px-5 py-16">
      <h2 className="display text-4xl sm:text-5xl">{t('faq.title')}</h2>
      <Accordion type="single" collapsible className="mt-8">
        {t('faq.items', { returnObjects: true }).map(({ q, a }) => (
          <AccordionItem key={q} value={q}>
            <AccordionTrigger className="text-base hover:no-underline">{q}</AccordionTrigger>
            <AccordionContent className="text-base text-muted-foreground">{a}</AccordionContent>
          </AccordionItem>
        ))}
      </Accordion>
    </section>
  )
}

export function CallToAction() {
  const { t, i18n } = useTranslation()
  return (
    <section className="mx-auto max-w-6xl px-5 pb-20">
      <div className="beans-cta relative overflow-hidden rounded-[28px]">
        <div className="relative px-6 py-20 text-center">
          <Logo className="mx-auto size-16 drop-shadow-2xl" />
          <h2 className="display mt-6 text-4xl text-foreground sm:text-6xl">{t('cta.title')}</h2>
          <p className="mx-auto mt-4 max-w-md text-muted-foreground">{t('cta.body')}</p>
          <Button asChild size="lg" className="mt-8 h-12 rounded-full px-7 text-base">
            <Link to={downloadPath(i18n.language)}>{t('nav.download')}</Link>
          </Button>
        </div>
      </div>
    </section>
  )
}

export function Footer() {
  const { t, i18n } = useTranslation()
  return (
    <footer className="border-t">
      <div className="mx-auto flex max-w-6xl flex-col items-center justify-between gap-4 px-5 py-8 text-sm text-muted-foreground/80 sm:flex-row">
        <div className="flex items-center gap-2">
          <Logo className="size-5" />
          <span>© {new Date().getFullYear()} Beans</span>
        </div>
        <nav className="flex flex-wrap justify-center gap-x-6 gap-y-3">
          <a href="/privacy.html" className="hover:text-foreground">{t('footer.privacy')}</a>
          <a href="/support.html" className="hover:text-foreground">{t('footer.support')}</a>
          <SectionLink id="faq" className="hover:text-foreground">{t('footer.faq')}</SectionLink>
          <Link to={docsPath(i18n.language)} className="hover:text-foreground">{t('nav.docs')}</Link>
          <Link to={downloadPath(i18n.language)} className="hover:text-foreground">{t('footer.download')}</Link>
          <LanguageLink className="hover:text-foreground" />
        </nav>
      </div>
    </footer>
  )
}
