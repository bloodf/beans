import { HeadContent, Scripts, createRootRoute, useLocation } from '@tanstack/react-router'
import { I18nextProvider } from 'react-i18next'

import { htmlLang, i18nFor, languageOf } from '#/i18n'

import appCss from '../styles.css?url'

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      {
        name: 'theme-color',
        content: '#fafafb',
        media: '(prefers-color-scheme: light)',
      },
      {
        name: 'theme-color',
        content: '#0f0f12',
        media: '(prefers-color-scheme: dark)',
      },
      { property: 'og:type', content: 'website' },
      {
        property: 'og:image',
        content: 'https://usebeans.app/brand/beans-social-v2.png',
      },
      { property: 'og:image:width', content: '1730' },
      { property: 'og:image:height', content: '909' },
      {
        property: 'og:image:alt',
        content: 'Beans — Your AI. Your space. Coral Beans sculpture on ivory.',
      },
      {
        name: 'twitter:image',
        content: 'https://usebeans.app/brand/beans-social-v2.png',
      },
      { name: 'twitter:card', content: 'summary_large_image' },
    ],
    links: [
      { rel: 'stylesheet', href: appCss },
      { rel: 'icon', href: '/favicon.png', type: 'image/png', sizes: '64x64' },
    ],
  }),
  shellComponent: RootDocument,
})

/// Keeps `.dark` on <html> with the system's appearance, from the first paint, for the docs'
/// styles (Fumadocs keys code colors off the class; the site's own use the media query). It lives
/// in the shell, which is rendered on the server and hydrated, never created on the client, where
/// React warns about script tags.
const followAppearance = `{const m=matchMedia('(prefers-color-scheme: dark)'),a=()=>document.documentElement.classList.toggle('dark',m.matches);a();m.addEventListener('change',a)}`

function RootDocument({ children }: { children: React.ReactNode }) {
  const lng = languageOf(useLocation({ select: (location) => location.pathname }))
  return (
    // The script below adds a class to <html> before React hydrates it.
    <html lang={htmlLang[lng]} suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: followAppearance }} />
        <HeadContent />
      </head>
      <body>
        <I18nextProvider i18n={i18nFor(lng)}>{children}</I18nextProvider>
        <Scripts />
      </body>
    </html>
  )
}
