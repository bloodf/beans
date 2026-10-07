import { createFileRoute, notFound } from '@tanstack/react-router'
import { ComparisonPage, comparisonHead, isComparisonSlug } from '#/components/site/comparison'
export const Route = createFileRoute('/compare/$product')({
  loader: ({ params }) => {
    if (!isComparisonSlug(params.product)) throw notFound()
    return { slug: params.product }
  },
  head: ({ loaderData }) => comparisonHead('en', loaderData?.slug),
  component: Page,
})
function Page() {
  const { slug } = Route.useLoaderData()
  return <ComparisonPage slug={slug} />
}
