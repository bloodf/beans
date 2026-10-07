import { createFileRoute } from '@tanstack/react-router'
import { ComparisonPage, comparisonHead } from '#/components/site/comparison'
export const Route = createFileRoute('/compare/')({
  head: () => comparisonHead('en'),
  component: ComparisonPage,
})
