import { createFileRoute, notFound } from '@tanstack/react-router'
import {
  ComparisonPage,
  comparisonHead,
  isComparisonSlug,
} from '#/components/site/comparison'
import { languageForLocale } from '#/i18n'
export const Route = createFileRoute('/$locale_/compare/$product')({
  loader: ({ params }) => {
    const language = languageForLocale(params.locale)
    if (!language || !isComparisonSlug(params.product)) throw notFound()
    return { language, slug: params.product }
  },
  head: ({ loaderData }) =>
    loaderData ? comparisonHead(loaderData.language, loaderData.slug) : {},
  component: () => <ComparisonPage slug={Route.useLoaderData().slug} />,
})
