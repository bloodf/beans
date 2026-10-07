export const comparisonProducts = {
  openclaw: {
    name: 'OpenClaw',
    sources: [
      'https://docs.openclaw.ai/help/faq/what-is-openclaw',
      'https://docs.openclaw.ai/concepts/multi-agent',
      'https://github.com/openclaw/openclaw',
    ],
  },
  hermes: {
    name: 'Hermes Agent',
    sources: [
      'https://hermes-agent.nousresearch.com/docs/',
      'https://hermes-agent.nousresearch.com/docs/user-guide/bot-mode',
      'https://hermes-agent.nousresearch.com/docs/integrations/nous-portal',
    ],
  },
  'grok-bot': { name: 'Grok Bot', sources: ['https://x.ai/bot'] },
  dots: {
    name: 'OpenAI Dots',
    sources: [
      'https://openai.com/index/introducing-dots/',
      'https://help.openai.com/en/articles/20001529-dots-privacy-security-and-safety-faqs',
      'https://openai.com/index/how-we-build-safety-security-and-privacy-into-dots/',
    ],
  },
  openbot: {
    name: 'OpenBot',
    sources: [
      'https://openbot.run/',
      'https://openbot.run/guides/openbot-hosted-servers',
      'https://openbot.run/news/one-agent-many-providers',
      'https://openbot.run/news/your-work-stays-on-your-computer',
    ],
  },
  'claude-code': {
    name: 'Claude Code',
    sources: [
      'https://code.claude.com/docs/en/overview',
      'https://code.claude.com/docs/en/security',
      'https://code.claude.com/docs/en/costs',
      'https://code.claude.com/docs/en/permissions',
    ],
  },
}
export type ComparisonSlug = keyof typeof comparisonProducts
export const comparisonSlugs: ComparisonSlug[] = [
  'openclaw',
  'hermes',
  'grok-bot',
  'dots',
  'openbot',
  'claude-code',
]
export const comparisonNames = comparisonSlugs.map((slug) => ({
  slug,
  name: comparisonProducts[slug].name,
}))
export function isComparisonSlug(value: string): value is ComparisonSlug {
  return Object.hasOwn(comparisonProducts, value)
}
