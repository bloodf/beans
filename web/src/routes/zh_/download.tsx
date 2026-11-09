import { createFileRoute } from '@tanstack/react-router'

import { Download, downloadHead, latestMacRelease } from '#/components/site/download'

export const Route = createFileRoute('/zh_/download')({
  loader: () => latestMacRelease(),
  head: () => downloadHead('zh'),
  component: () => <Download release={Route.useLoaderData()} />,
})
