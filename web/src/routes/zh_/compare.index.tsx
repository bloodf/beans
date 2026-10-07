import { createFileRoute } from '@tanstack/react-router'
import { ComparisonPage, comparisonHead } from '#/components/site/comparison'
export const Route = createFileRoute('/zh_/compare/')({
  head: () => comparisonHead('zh'),
  component: ComparisonPage,
})
