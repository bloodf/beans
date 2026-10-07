import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { ArrowLeft, ArrowUpRight, Check, Minus } from 'lucide-react'
import { Nav, SITE, downloadPath } from './nav'
import { MarketingFooter } from './marketing'
import { MotionExperience } from './motion'
import {
  currentLanguage,
  i18nFor,
  languages,
  localizedPath,
  type Language,
} from '#/i18n'

import {
  comparisonProducts as products,
  comparisonSlugs as slugs,
  type ComparisonSlug,
} from './comparison-products'
export { isComparisonSlug, type ComparisonSlug } from './comparison-products'
export function comparisonHead(lng: Language, slug?: ComparisonSlug) {
  const t = i18nFor(lng).t
  const copy = t('comparison', { returnObjects: true })
  const title = slug
    ? t('comparison.versus', { product: products[slug].name })
    : copy.title
  const description = slug ? copy.products[slug].intro : copy.description
  const page = `/compare${slug ? '/' + slug : ''}`
  return {
    meta: [
      { title },
      { name: 'description', content: description },
      { property: 'og:title', content: title },
      { property: 'og:description', content: description },
    ],
    links: [
      { rel: 'canonical', href: SITE + localizedPath(lng, page) },
      ...languages.map((other) => ({
        rel: 'alternate',
        hrefLang: other,
        href: SITE + localizedPath(other, page),
      })),
    ],
  }
}
export function ComparisonPage({ slug }: { slug?: ComparisonSlug }) {
  const { t, i18n } = useTranslation()
  const c = t('comparison', { returnObjects: true })
  const lng = currentLanguage(i18n.language)
  const selected = slug ? [slug] : slugs
  const product = slug ? products[slug] : undefined
  const copy = slug ? c.products[slug] : undefined
  const sections = [
    { id: 'overview', title: c.overview },
    { id: 'matrix', title: c.matrix },
    { id: 'details', title: c.details },
    { id: 'costs', title: c.costTitle },
    { id: 'choose', title: c.choose },
    { id: 'questions', title: c.faq },
    { id: 'sources', title: c.sources },
  ]
  return (
    <MotionExperience>
      <Nav />
      <main className="comparison-page story-section">
        <header className="comparison-hero motion-island">
          <div className="gradient-wash" aria-hidden="true" />
          <img
            className="comparison-mark"
            src="/brand/beans-mark.svg"
            alt=""
            aria-hidden="true"
            width="300"
            height="300"
          />
          <Link
            className="text-link"
            to={localizedPath(lng, product ? '/compare' : '/')}
          >
            <ArrowLeft size={17} />
            {product ? t('marketing.compareAll') : 'Beans'}
          </Link>
          <h1 className="display">
            {product
              ? t('comparison.versus', { product: product.name })
              : c.title}
          </h1>
          <p className="comparison-intro">{copy?.intro ?? c.description}</p>
          <Link className="marketing-button" to={downloadPath(lng)}>
            {t('marketing.download')}
            <ArrowUpRight size={18} />
          </Link>
        </header>
        {!slug ? (
          <section className="comparison-bento" aria-label={c.overview}>
            <article className="comparison-bento-beans">
              <div>
                <h2 className="display">Beans</h2>
                <p>{c.costBody}</p>
                <Link className="text-link" to={downloadPath(lng)}>
                  {t('marketing.download')}
                  <ArrowUpRight size={18} />
                </Link>
              </div>
              <ProsCons
                pros={c.pros}
                cons={c.cons}
                positive={c.beansFit}
                negative={c.beansTradeoff}
              />
            </article>
            {slugs.map((key) => (
              <article className="comparison-bento-card" key={key}>
                <h2>{products[key].name}</h2>
                <ProsCons
                  pros={c.pros}
                  cons={c.cons}
                  positive={c.products[key].good}
                  negative={c.products[key].tradeoff}
                />
                <div className="comparison-bento-cost">
                  <h3>{t('marketing.costs')}</h3>
                  <p>{c.products[key].other[11]}</p>
                </div>
                <Link
                  className="text-link"
                  to={localizedPath(lng, '/compare/' + key)}
                >
                  {c.readComparison}
                  <ArrowUpRight size={18} />
                </Link>
              </article>
            ))}
          </section>
        ) : (
          <>
            <nav className="comparison-contents" aria-label={c.contents}>
              {sections.map((section) => (
                <a key={section.id} href={`#${section.id}`}>
                  {section.title}
                </a>
              ))}
            </nav>
            <section id="overview" className="comparison-section">
              <h2 className="display">{c.overview}</h2>
              <div className="comparison-overview">
                <article>
                  <h3>Beans</h3>
                  <ProsCons
                    pros={c.pros}
                    cons={c.cons}
                    positive={c.beansFit}
                    negative={c.beansTradeoff}
                  />
                </article>
                {selected.map((key) => (
                  <article key={key}>
                    <h3>{products[key].name}</h3>
                    <ProsCons
                      pros={c.pros}
                      cons={c.cons}
                      positive={c.products[key].good}
                      negative={c.products[key].tradeoff}
                    />
                    <Link
                      className="text-link"
                      to={localizedPath(lng, '/compare/' + key)}
                    >
                      {t('comparison.versus', { product: products[key].name })}
                      <ArrowUpRight size={17} />
                    </Link>
                  </article>
                ))}
              </div>
            </section>
            <section id="matrix" className="comparison-section">
              <h2 className="display">{c.matrix}</h2>
              <p className="comparison-section-intro">{c.matrixBody}</p>
              <div className="comparison-dimensions">
                {c.labels.map((label, index) => (
                  <article className="comparison-dimension" key={label}>
                    <h3>{label}</h3>
                    <dl>
                      <div>
                        <dt>Beans</dt>
                        <dd>{c.beans[index]}</dd>
                      </div>
                      {selected.map((key) => (
                        <div key={key}>
                          <dt>{products[key].name}</dt>
                          <dd>{c.products[key].other[index]}</dd>
                        </div>
                      ))}
                    </dl>
                  </article>
                ))}
              </div>
            </section>
            <section id="details" className="comparison-section">
              <h2 className="display">{c.details}</h2>
              <div className="comparison-editorial">
                {c.topics.map((topic) => (
                  <article key={topic.title}>
                    <h3>{topic.title}</h3>
                    <p>{topic.body}</p>
                  </article>
                ))}
              </div>
            </section>
            <section
              id="costs"
              className="comparison-costs comparison-section motion-island"
            >
              <div className="gradient-wash" aria-hidden="true" />
              <h2 className="display">{c.costTitle}</h2>
              <p className="comparison-section-intro">{c.costBody}</p>
              <dl className="comparison-cost-breakdown">
                <div>
                  <dt>{c.appPrice}</dt>
                  <dd>$0</dd>
                </div>
                <div>
                  <dt>{c.providerPrice}</dt>
                  <dd>{c.providerCost}</dd>
                </div>
                <div>
                  <dt>{c.machinePrice}</dt>
                  <dd>{c.machineCost}</dd>
                </div>
              </dl>
              {selected.map((key) => (
                <p className="comparison-other-cost" key={key}>
                  <strong>{products[key].name}</strong>{' '}
                  {c.products[key].other[11]}
                </p>
              ))}
            </section>
            <section id="choose" className="comparison-section">
              <h2 className="display">{c.choose}</h2>
              <div className="comparison-choice">
                <article>
                  <h3>{c.chooseBeans}</h3>
                  <p>{c.beansFit}</p>
                  <Link className="text-link" to={downloadPath(lng)}>
                    {t('marketing.download')}
                    <ArrowUpRight size={17} />
                  </Link>
                </article>
                {selected.map((key) => (
                  <article key={key}>
                    <h3>
                      {t('comparison.chooseOther', {
                        product: products[key].name,
                      })}
                    </h3>
                    <p>{c.products[key].good}</p>
                    <a className="text-link" href={products[key].sources[0]}>
                      {products[key].name}
                      <ArrowUpRight size={17} />
                    </a>
                  </article>
                ))}
              </div>
            </section>
            <section
              id="questions"
              className="comparison-section comparison-faq"
            >
              <h2 className="display">{c.faq}</h2>
              {c.questions.map((item) => (
                <details key={item.q}>
                  <summary>{item.q}</summary>
                  <p>{item.a}</p>
                </details>
              ))}
            </section>
            <section
              id="sources"
              className="comparison-section comparison-sources"
            >
              <h2>{c.sources}</h2>
              <p>{c.freshness}</p>
              <a href="https://github.com/bloodf/beans/blob/feat/durindoor-fresh-start-releases/ARCHITECTURE.md">
                {c.architecture}
                <ArrowUpRight size={15} />
              </a>
              {selected.map((key) => (
                <div key={key}>
                  <h3>{products[key].name}</h3>
                  {products[key].sources.map((source, index) => (
                    <a key={source} href={source}>
                      {c.official} · {products[key].name}
                      {index ? ` (${index + 1})` : ''}
                      <ArrowUpRight size={15} />
                    </a>
                  ))}
                </div>
              ))}
            </section>
            <section className="comparison-section comparison-final motion-island">
              <div className="gradient-wash" aria-hidden="true" />
              <h2 className="display">{c.start}</h2>
              <p>{c.startBody}</p>
              <div>
                <Link
                  className="marketing-button"
                  to={localizedPath(lng, '/')}
                  hash="turns"
                >
                  {c.demo}
                  <ArrowUpRight size={17} />
                </Link>
                <Link className="text-link" to={downloadPath(lng)}>
                  {t('marketing.download')}
                  <ArrowUpRight size={17} />
                </Link>
              </div>
            </section>
            <section className="comparison-section">
              <h2>{c.more}</h2>
              <div className="comparison-grid">
                {slugs
                  .filter((key) => key !== slug)
                  .map((key) => (
                    <Link key={key} to={localizedPath(lng, '/compare/' + key)}>
                      <span>
                        {t('comparison.versus', {
                          product: products[key].name,
                        })}
                      </span>
                      <ArrowUpRight size={22} />
                    </Link>
                  ))}
              </div>
            </section>
          </>
        )}
      </main>
      <MarketingFooter />
    </MotionExperience>
  )
}

function ProsCons({
  pros,
  cons,
  positive,
  negative,
}: {
  pros: string
  cons: string
  positive: string
  negative: string
}) {
  return (
    <dl className="comparison-pros-cons">
      <div>
        <dt>
          <Check size={16} aria-hidden="true" />
          {pros}
        </dt>
        <dd>{positive}</dd>
      </div>
      <div>
        <dt>
          <Minus size={16} aria-hidden="true" />
          {cons}
        </dt>
        <dd>{negative}</dd>
      </div>
    </dl>
  )
}
