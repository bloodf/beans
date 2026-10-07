import { createFileRoute, notFound } from '@tanstack/react-router'
import {
  Download,
  downloadHead,
  latestMacRelease,
} from '#/components/site/download'
import { languageForLocale } from '#/i18n'
export const Route = createFileRoute('/$locale_/download')({
  loader: async ({ params }) => {
    const language = languageForLocale(params.locale)
    if (!language) throw notFound()
    return { language, release: await latestMacRelease() }
  },
  head: ({ loaderData }) =>
    loaderData ? downloadHead(loaderData.language) : {},
  component: () => <Download release={Route.useLoaderData().release} />,
})
