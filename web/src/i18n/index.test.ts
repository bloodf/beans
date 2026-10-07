import { describe, expect, test } from 'bun:test'
import {
  comparisonProducts,
  comparisonSlugs,
  isComparisonSlug,
} from '../components/site/comparison-products'
import {
  htmlLang,
  i18nFor,
  languageForLocale,
  languageOf,
  languages,
  pathIn,
} from './index'

function leaves(value: unknown, path = ''): Map<string, string> {
  const result = new Map<string, string>()
  if (typeof value === 'string') result.set(path, value)
  else if (value && typeof value === 'object') {
    for (const [key, child] of Object.entries(value)) {
      for (const [leaf, text] of leaves(child, path ? `${path}.${key}` : key))
        result.set(leaf, text)
    }
  }
  return result
}
function tokens(value: string) {
  return [...value.matchAll(/\{\{[^}]+\}\}|<\/?[\w]+\s*\/?\s*>/g)]
    .map((match) => match[0])
    .sort()
}
describe('published website languages', () => {
  const english = leaves(i18nFor('en').getResourceBundle('en', 'translation'))
  for (const language of languages) {
    test(`${language}: complete messages and preserved interpolation/markup`, () => {
      const translated = leaves(
        i18nFor(language).getResourceBundle(language, 'translation'),
      )
      expect([...translated.keys()].sort()).toEqual([...english.keys()].sort())
      for (const [key, source] of english) {
        const value = translated.get(key)
        expect(value, key).toBeTruthy()
        expect(tokens(value ?? ''), key).toEqual(tokens(source))
      }
      expect(
        i18nFor(language).t('download.version', { version: '1.2.3' }),
      ).toContain('1.2.3')
      expect(
        i18nFor(language).t('comparison.versus', { product: 'OpenBot' }),
      ).toContain('OpenBot')
      expect(htmlLang[language]).toBeTruthy()
    })
    test(`${language}: same-page URL preserves locale and comparison slug`, () => {
      for (const page of ['/', '/download', '/compare', '/compare/openbot']) {
        const localized = pathIn(language, page)
        expect(languageOf(localized)).toBe(language)
        expect(pathIn('en', localized)).toBe(page)
        for (const other of languages)
          expect(pathIn(other, localized)).toBe(pathIn(other, page))
      }
    })
  }
  test('unpublished languages are rejected; English docs remain reachable', () => {
    expect(languageForLocale('fr')).toBeUndefined()
    expect(languageForLocale('pt-br')).toBe('pt-BR')
    expect(pathIn('de', '/zh/docs/providers')).toBe('/docs/providers')
    expect(pathIn('zh', '/docs/providers')).toBe('/zh/docs/providers')
  })
  test('fixed-language instances do not change one another', () => {
    const englishTitle = i18nFor('en').t('hero.accessibleTitle')
    const portugueseTitle = i18nFor('pt-BR').t('hero.accessibleTitle')
    expect(portugueseTitle).not.toBe(englishTitle)
    expect(i18nFor('en').t('hero.accessibleTitle')).toBe(englishTitle)
  })
})

describe('published comparisons', () => {
  for (const language of languages) {
    test(`${language}: each published product has every matrix row`, () => {
      const copy = i18nFor(language).t('comparison', { returnObjects: true })
      expect(copy.labels).toHaveLength(12)
      expect(copy.beans).toHaveLength(copy.labels.length)
      expect(Object.keys(copy.products).sort()).toEqual(
        [...comparisonSlugs].sort(),
      )
      for (const slug of comparisonSlugs) {
        expect(isComparisonSlug(slug)).toBe(true)
        expect(copy.products[slug].other).toHaveLength(copy.labels.length)
        expect(copy.products[slug].intro).toBeTruthy()
        expect(copy.products[slug].good).toBeTruthy()
        expect(comparisonProducts[slug].sources.length).toBeGreaterThan(0)
      }
    })
  }
  test('unknown products are not published comparisons', () => {
    expect(isComparisonSlug('missing')).toBe(false)
  })
})
