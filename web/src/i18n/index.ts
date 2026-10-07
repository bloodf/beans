import { createInstance, type i18n } from 'i18next'
import { initReactI18next } from 'react-i18next'
import { en } from './en'
import { zh } from './zh'
import { enMarketing, zhMarketing } from './marketing'
import ptBR from './locales/pt-BR.json'
import de from './locales/de.json'
import es from './locales/es.json'
import ja from './locales/ja.json'
import comparisonEn from './locales/comparison-en.json'
import comparisonZh from './locales/comparison-zh.json'
import comparisonPtBR from './locales/comparison-pt-BR.json'
import comparisonDe from './locales/comparison-de.json'
import comparisonEs from './locales/comparison-es.json'
import comparisonJa from './locales/comparison-ja.json'

export const languages = ['en', 'pt-BR', 'zh', 'de', 'es', 'ja'] as const
export type Language = (typeof languages)[number]
export type DocsLanguage = 'en' | 'zh'
export const paths: Record<Language, string> = {
  en: '/',
  'pt-BR': '/pt-br',
  zh: '/zh',
  de: '/de',
  es: '/es',
  ja: '/ja',
}
export const names: Record<Language, string> = {
  en: 'English',
  'pt-BR': 'Português (Brasil)',
  zh: '简体中文',
  de: 'Deutsch',
  es: 'Español',
  ja: '日本語',
}
export const htmlLang: Record<Language, string> = {
  en: 'en',
  'pt-BR': 'pt-BR',
  zh: 'zh-Hans',
  de: 'de',
  es: 'es',
  ja: 'ja',
}
const english = { ...en, marketing: enMarketing, comparison: comparisonEn }
export type SiteMessages = typeof english
const resources = {
  en: { translation: english },
  'pt-BR': {
    translation: { ...ptBR, comparison: comparisonPtBR } satisfies SiteMessages,
  },
  zh: {
    translation: {
      ...zh,
      marketing: zhMarketing,
      comparison: comparisonZh,
    } satisfies SiteMessages,
  },
  de: {
    translation: { ...de, comparison: comparisonDe } satisfies SiteMessages,
  },
  es: {
    translation: { ...es, comparison: comparisonEs } satisfies SiteMessages,
  },
  ja: {
    translation: { ...ja, comparison: comparisonJa } satisfies SiteMessages,
  },
}
declare module 'i18next' {
  interface CustomTypeOptions {
    resources: (typeof resources)['en']
  }
}
export function isLanguage(value: string): value is Language {
  return languages.some((language) => language === value)
}
export function currentLanguage(value: string): Language {
  return isLanguage(value) ? value : 'en'
}
export function languageOf(pathname: string): Language {
  return (
    languages.find(
      (language) =>
        language !== 'en' &&
        (pathname === paths[language] ||
          pathname.startsWith(paths[language] + '/')),
    ) ?? 'en'
  )
}
export function languageForLocale(locale: string): Language | undefined {
  return languages.find((language) => paths[language] === '/' + locale)
}
export function pathIn(lng: Language, pathname: string): string {
  const original = languageOf(pathname)
  const page =
    original === 'en' ? pathname : pathname.slice(paths[original].length) || '/'
  // The product docs are published in English and Chinese.
  if (page === '/docs' || page.startsWith('/docs/'))
    return (lng === 'zh' ? '/zh' : '') + page
  return lng === 'en' ? page : page === '/' ? paths[lng] : paths[lng] + page
}
export function localizedPath(lng: string, page: string): string {
  return pathIn(currentLanguage(lng), page)
}
const instances = new Map<Language, i18n>()
export function i18nFor(lng: Language): i18n {
  let instance = instances.get(lng)
  if (!instance) {
    instance = createInstance()
    instance.use(initReactI18next).init({
      lng,
      supportedLngs: [...languages],
      load: 'currentOnly',
      fallbackLng: 'en',
      resources,
      initAsync: false,
      interpolation: { escapeValue: false },
    })
    instances.set(lng, instance)
  }
  return instance
}
