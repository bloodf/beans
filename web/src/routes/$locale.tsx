import { createFileRoute, notFound } from '@tanstack/react-router'
import { Home, homeHead } from '#/components/site/home'
import { languageForLocale } from '#/i18n'
export const Route = createFileRoute('/$locale')({
  loader: ({ params }) => {
    const language = languageForLocale(params.locale)
    if (!language) throw notFound()
    return { language }
  },
  head: ({ loaderData }) => (loaderData ? homeHead(loaderData.language) : {}),
  component: Home,
})
