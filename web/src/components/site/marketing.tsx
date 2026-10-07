import { localizedPath } from '#/i18n'
import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import {
  ArrowUpRight,
  CalendarDays,
  Check,
  Code2,
  FileText,
  Folder,
  GitBranch,
  LockKeyhole,
  Monitor,
  Plug,
  Smartphone,
} from 'lucide-react'
import { BotAvatar } from './work-examples'
import { useMarketingCopy } from './marketing-copy'
import { docsPath, downloadPath, SectionLink } from './nav'

export const providerNames = [
  { name: 'ChatGPT', icon: 'openai', type: 'subscription' },
  { name: 'Grok', icon: 'grok', type: 'subscription' },
  { name: 'Claude', icon: 'claude', type: 'api' },
  { name: 'DeepSeek', icon: 'deepseek', type: 'api' },
  { name: 'OpenCode', icon: 'opencode', type: 'api' },
] as const

export function BentoFeatures() {
  const c = useMarketingCopy()
  const visuals = [
    <div className="bento-conversation" key="team">
      {[c.read, c.handoff, c.review].map((line, index) => (
        <div key={line}>
          <BotAvatar bot={index} />
          <span>
            <b>{['Scout', 'Builder', 'Chef'][index]}</b>
            <small>{line}</small>
          </span>
          <Check size={17} />
        </div>
      ))}
    </div>,
    <div className="bento-files" key="files">
      <div>
        <Folder size={18} />
        {c.files}
      </div>
      <p>
        <FileText size={18} />
        brief.md <span>+12</span>
      </p>
      <p>
        <Code2 size={18} />
        landing.tsx <span>+38 −6</span>
      </p>
      <p>
        <Check size={18} />
        {c.edited}
      </p>
    </div>,
    <div className="bento-memory" key="memory">
      <span>{c.remembered}</span>
      <p>“{c.memory}”</p>
      <div>
        <BotAvatar bot={0} />
        <b>Chef</b>
        <Check size={18} />
      </div>
    </div>,
    <div className="bento-schedule" key="routine">
      <CalendarDays size={32} />
      <strong>{c.routine}</strong>
      <span>{c.schedule}</span>
      <div className="week-days" aria-hidden="true">
        {['M', 'T', 'W', 'T', 'F', 'S', 'S'].map((day, index) => (
          <i key={index} className={index === 0 ? 'scheduled-day' : ''}>
            {day}
          </i>
        ))}
      </div>
    </div>,
    <div className="bento-connections" key="tools">
      <div className="connection-icons">
        <GitBranch />
        <Plug />
        <Code2 />
      </div>
      <div>
        <LockKeyhole size={15} />
        {c.permission}
      </div>
      <small>{c.permissionBody}</small>
    </div>,
    <div className="bento-devices" key="devices">
      <Monitor size={56} strokeWidth={1.3} />
      <div className="sync-path">
        <LockKeyhole size={20} />
        <i />
      </div>
      <Smartphone size={42} strokeWidth={1.3} />
      <span>{c.devices}</span>
    </div>,
  ]
  return (
    <section id="tools" className="story-section">
      <div className="story-heading">
        <h2 className="display">{c.featuresTitle}</h2>
        <p>{c.featuresBody}</p>
      </div>
      <div className="bento-grid">
        {c.cards.map((card, index) => (
          <article
            key={card.title}
            className={`bento-card bento-${index} motion-island`}
          >
            <div className="bento-art" aria-hidden="true">
              {visuals[index]}
            </div>
            <div className="bento-copy">
              <h3>{card.title}</h3>
              <p>{card.body}</p>
            </div>
          </article>
        ))}
      </div>
      <p className="demo-disclaimer">{c.preview}</p>
    </section>
  )
}
export function Providers() {
  const c = useMarketingCopy()
  const { i18n } = useTranslation()
  return (
    <section id="providers" className="story-section provider-section">
      <div className="story-heading">
        <h2 className="display">{c.providersTitle}</h2>
        <p>{c.providersBody}</p>
      </div>
      <div className="provider-grid">
        {providerNames.map((provider) => (
          <div key={provider.name} className="provider-tile">
            <img
              src={`/providers/${provider.icon}.svg`}
              alt=""
              width="40"
              height="40"
              loading="lazy"
            />
            <strong>{provider.name}</strong>
            <span>{c[provider.type]}</span>
          </div>
        ))}
      </div>
      <div className="provider-custom">
        <Code2 size={24} />
        <div>
          <strong>{c.compatible}</strong>
          <p>{c.compatibleBody}</p>
        </div>
        <Link to={docsPath(i18n.language, 'providers')}>
          {c.providerDocs}
          <ArrowUpRight size={17} />
        </Link>
      </div>
    </section>
  )
}
export function Pricing() {
  const c = useMarketingCopy()
  const { i18n } = useTranslation()
  return (
    <section id="costs" className="story-section pricing-section">
      <div className="story-heading">
        <h2 className="display">{c.pricingTitle}</h2>
        <p>{c.priceBody}</p>
      </div>
      <div className="pricing-layout">
        <div className="free-panel">
          <div className="free-price">
            $0<span>{c.priceLabel}</span>
          </div>
          <ul>
            {c.included.map((item) => (
              <li key={item}>
                <Check size={18} />
                {item}
              </li>
            ))}
          </ul>
          <Link className="marketing-button" to={downloadPath(i18n.language)}>
            {c.download}
            <ArrowUpRight size={18} />
          </Link>
        </div>
        <div className="provider-costs">
          <div>
            <h3>{c.subscriptionTitle}</h3>
            <p>{c.subscriptionBody}</p>
          </div>
          <div>
            <h3>{c.apiTitle}</h3>
            <p>{c.apiBody}</p>
          </div>
          <p className="cost-note">{c.costNote}</p>
        </div>
      </div>
    </section>
  )
}
export const comparisonNames = [
  { slug: 'openbot', name: 'OpenBot' },
  { slug: 'grok-bot', name: 'Grok Bot' },
  { slug: 'claude-code', name: 'Claude Code' },
] as const
export function ComparisonLinks() {
  const c = useMarketingCopy()
  const { i18n } = useTranslation()
  const base = localizedPath(i18n.language, '/compare')
  return (
    <section id="compare" className="story-section comparison-links">
      <div className="story-heading">
        <h2 className="display">{c.compareTitle}</h2>
        <p>{c.compareBody}</p>
      </div>
      <div className="comparison-grid">
        {comparisonNames.map((item) => (
          <Link
            to={localizedPath(i18n.language, `/compare/${item.slug}`)}
            key={item.slug}
          >
            <span>
              Beans <small>vs</small>
              <br />
              {item.name}
            </span>
            <ArrowUpRight size={24} />
          </Link>
        ))}
      </div>
      <Link className="text-link" to={base}>
        {c.compareAll}
        <ArrowUpRight size={17} />
      </Link>
    </section>
  )
}
export function MarketingFooter() {
  const c = useMarketingCopy()
  const { i18n } = useTranslation()
  const base = localizedPath(i18n.language, '/compare')
  return (
    <footer className="marketing-footer">
      <div className="footer-inner">
        <div className="footer-brand">
          <img
            src="/brand/beans-logo.svg"
            alt="Beans"
            className="dark:hidden"
            width="150"
            height="51"
          />
          <img
            src="/brand/beans-logo-light.svg"
            alt="Beans"
            className="hidden dark:block"
            width="150"
            height="51"
          />
          <p>{c.footerBody}</p>
          <Link to={downloadPath(i18n.language)}>
            {c.download}
            <ArrowUpRight size={17} />
          </Link>
        </div>
        <nav aria-label={c.product}>
          <h2>{c.product}</h2>
          <SectionLink id="tools">{c.features}</SectionLink>
          <SectionLink id="costs">{c.costs}</SectionLink>
          <SectionLink id="providers">{c.providers}</SectionLink>
          <a href="https://github.com/bloodf/beans/releases">{c.releases}</a>
        </nav>
        <nav aria-label={c.resources}>
          <h2>{c.resources}</h2>
          <Link to={docsPath(i18n.language)}>{c.docs}</Link>
          <a href="https://github.com/bloodf/beans">{c.source}</a>
          <a href="/support.html">{c.support}</a>
          <a href="/privacy.html">{c.privacy}</a>
        </nav>
        <nav aria-label={c.providers}>
          <h2>{c.providers}</h2>
          {providerNames.map((item) => (
            <Link key={item.name} to={docsPath(i18n.language, 'providers')}>
              {item.name}
            </Link>
          ))}
        </nav>
        <nav aria-label={c.comparisons}>
          <h2>{c.comparisons}</h2>
          {comparisonNames.map((item) => (
            <Link
              key={item.slug}
              to={localizedPath(i18n.language, `/compare/${item.slug}`)}
            >
              Beans vs {item.name}
            </Link>
          ))}
          <Link to={base}>{c.compareAll}</Link>
        </nav>
        <div className="footer-bottom">
          <span>© {new Date().getFullYear()} Beans</span>
          <span>{c.footerNote}</span>
        </div>
      </div>
    </footer>
  )
}
