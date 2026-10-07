import { createFileRoute, notFound } from '@tanstack/react-router'
import { ComparisonPage, comparisonHead } from '#/components/site/comparison'
import { languageForLocale } from '#/i18n'
export const Route = createFileRoute('/$locale_/compare/')({
  loader: ({ params }) => {
    const language = languageForLocale(params.locale)
    if (!language) throw notFound()
    return { language }
  },
  head: ({ loaderData }) =>
    loaderData ? comparisonHead(loaderData.language) : {},
  component: ComparisonPage,
})
