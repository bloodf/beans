import { Link } from '@tanstack/react-router'
import { ArrowDownRight, ArrowUpRight, Brain, FilePen, Globe, Plug } from 'lucide-react'
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
import { BeanScene } from './bean-scene'

export function Hero() {
  const { t, i18n } = useTranslation()
  return (
    <section id="top" className="beans-hero">
      <div className="hero-copy">
      <h1 className="hero-title display">
        <Trans i18nKey="hero.title" components={{ accent: <span className="brush" /> }} />
      </h1>
      <p className="mt-7 max-w-lg text-lg text-pretty text-muted-foreground sm:text-xl">
        {t('hero.body')}
      </p>
      <div className="mt-9 flex flex-col gap-3 sm:flex-row">
        <Button asChild size="lg" className="h-12 rounded-full px-7 text-base">
          <Link viewTransition to={downloadPath(i18n.language)}>{t('nav.download')}<ArrowUpRight size={18} aria-hidden="true" /></Link>
        </Button>
        <Button asChild size="lg" variant="outline" className="h-12 rounded-full bg-transparent px-6 text-base shadow-none hover:bg-foreground/5 dark:bg-transparent dark:hover:bg-foreground/5">
          <SectionLink id="turns">{t('hero.how')}<ArrowDownRight size={18} aria-hidden="true" /></SectionLink>
        </Button>
      </div>
      </div>
      <BeanScene />
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
    <section id={id} className={`story-section story-${id}`}>
      <div className="story-content">
        <div className="story-heading">
          <h2 className="display max-w-3xl text-4xl sm:text-5xl">{title}</h2>
          <p className="mt-5 max-w-3xl text-lg leading-relaxed text-muted-foreground">{body}</p>
        </div>
        {bare ? (
          children
        ) : (
          <div className="beans-stage">
            {id === 'relay' && <div className="relay-orbit" aria-hidden="true"><div /><div /><div /></div>}
            <div className="stage-visual">{children}</div>
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
        className="window-frame screen-window mx-auto w-full"
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
      <ul className="tool-list">
        {toolKinds.map(({ key, icon: Icon }) => (
          <li key={key} className="tool-row">
            <Icon className="tool-icon text-violet" strokeWidth={1.5} aria-hidden="true" />
            <div>
              <h3 className="text-2xl font-semibold">{t(`tools.kinds.${key}.title`)}</h3>
              <p className="mt-3 leading-relaxed text-muted-foreground">{t(`tools.kinds.${key}.body`)}</p>
            </div>
          </li>
        ))}
      </ul>
    </Stage>
  )
}

export function Chef() {
  const { t } = useTranslation()
  return (
    <section id="start" className="story-section story-start">
      <div className="setup-layout">
        <div className="story-heading">
          <h2 className="display mt-3 text-4xl sm:text-5xl">{t('chef.title')}</h2>
          <p className="mt-5 text-lg leading-relaxed text-muted-foreground">
            {t('chef.body')}
          </p>
        </div>
        <ol className="setup-list">
          {t('chef.steps', { returnObjects: true }).map(({ title, body }, i) => (
            <li key={title} className="setup-step">
              <span className="setup-number">{i + 1}</span>
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
    <section id="faq" className="story-section story-faq">
      <h2 className="story-heading display text-4xl sm:text-5xl">{t('faq.title')}</h2>
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
    <section className="story-section story-cta">
      <div className="beans-cta">
        <div className="cta-art" aria-hidden="true"><Logo className="cta-mark" /></div>
        <div className="cta-copy">
          <h2 className="display text-4xl text-foreground sm:text-6xl">{t('cta.title')}</h2>
          <p className="mx-auto mt-4 max-w-md text-muted-foreground">{t('cta.body')}</p>
          <Button asChild size="lg" className="mt-8 h-12 rounded-full px-7 text-base">
            <Link viewTransition to={downloadPath(i18n.language)}>{t('nav.download')}<ArrowUpRight size={18} aria-hidden="true" /></Link>
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
      <div className="mx-auto flex max-w-6xl flex-col items-center justify-between gap-4 px-5 py-8 text-sm text-muted-foreground sm:flex-row">
        <div className="flex items-center gap-2">
          <Logo className="size-5" />
          <span>© {new Date().getFullYear()} Beans</span>
        </div>
        <nav className="flex flex-wrap justify-center gap-x-6 gap-y-3">
          <a href="/privacy.html" className="hover:text-foreground">{t('footer.privacy')}</a>
          <a href="/support.html" className="hover:text-foreground">{t('footer.support')}</a>
          <SectionLink id="faq" className="hover:text-foreground">{t('footer.faq')}</SectionLink>
          <Link viewTransition to={docsPath(i18n.language)} className="hover:text-foreground">{t('nav.docs')}</Link>
          <Link viewTransition to={downloadPath(i18n.language)} className="hover:text-foreground">{t('footer.download')}</Link>
          <LanguageLink className="hover:text-foreground" />
        </nav>
      </div>
    </footer>
  )
}
