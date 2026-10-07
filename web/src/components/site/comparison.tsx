import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { ArrowLeft, ArrowUpRight } from 'lucide-react'
import { Nav, SITE, downloadPath } from './nav'
import { MarketingFooter, comparisonNames } from './marketing'
import { MotionExperience } from './motion'
import { useMarketingCopy } from './marketing-copy'
import type { Language } from '#/i18n'

const products = {
  openbot: {
    name: 'OpenBot',
    source: 'https://openbot.run/',
    en: {
      intro:
        'Two ways to run an AI team on your computer. Beans centers named bots, shared chats, and encrypted pairing. OpenBot brings existing agent tools into a shared desktop workspace.',
      other: [
        'Local desktop workspace',
        'Codex, Claude Code, Gemini, Grok and other agent tools',
        'Free app; provider plans are separate',
        'Agents with shared channels and handoffs',
      ],
      good: 'Choose OpenBot if you want existing agent tools such as Claude Code or Codex inside one workspace. Choose Beans if named bots, direct model connections, and encrypted device pairing fit how you work.',
    },
    zh: {
      intro:
        '两种在电脑上运行 AI 团队的方式。Beans 以命名智能体、群聊和加密配对为中心；OpenBot 将现有智能体工具放进共享桌面工作区。',
      other: [
        '本地桌面工作区',
        'Codex、Claude Code、Gemini、Grok 等工具',
        '应用免费，提供商套餐另付',
        '共享频道中的智能体和任务交接',
      ],
      good: '如果你想把 Claude Code 或 Codex 等工具放进一个工作区，可以考虑 OpenBot。如果命名智能体、直接模型连接和加密设备配对更适合你，可以考虑 Beans。',
    },
  },
  'grok-bot': {
    name: 'Grok Bot',
    source: 'https://x.ai/bot',
    en: {
      intro:
        'Your computer or a managed cloud computer? Beans runs bot tools on the desktop you assign. Grok Bot gives its bots a persistent cloud computer.',
      other: [
        'Persistent cloud computer',
        'Grok Bot service with eligible Cursor or Grok plans',
        'Eligible paid plan; included usage and additional billing',
        'Bots collaborate and run routines in a shared thread',
      ],
      good: 'Choose Grok Bot if you want a managed cloud computer that keeps working while your laptop is closed. Choose Beans if you want work on your own Runner and the freedom to choose connected model providers.',
    },
    zh: {
      intro:
        '自己的电脑，还是托管云电脑？Beans 在指定桌面设备上运行工具，Grok Bot 为智能体提供持久云电脑。',
      other: [
        '持久云电脑',
        '符合条件的 Cursor 或 Grok 套餐中的 Grok Bot 服务',
        '符合条件的付费套餐，含用量并可另行计费',
        '智能体在共享对话中协作并运行例行任务',
      ],
      good: '需要笔记本关机后仍能工作的托管云电脑，可以考虑 Grok Bot。想使用自己的运行设备并自由选择模型提供商，可以考虑 Beans。',
    },
  },
  'claude-code': {
    name: 'Claude Code',
    source: 'https://code.claude.com/docs/en/overview',
    en: {
      intro:
        'A general-purpose bot workspace or a coding-focused agent? Beans organizes named teammates in chats. Claude Code is built around reading code, editing files, and running development commands.',
      other: [
        'Local terminal/IDE/desktop, plus web and cloud sessions',
        'Claude subscription or Anthropic Console; selected surfaces support third-party providers',
        'Subscription or API access, depending on the surface',
        'Coding sessions, parallel agents, and development workflows',
      ],
      good: 'Choose Claude Code for a coding-first workflow inside your terminal, IDE, or Claude app. Choose Beans for named teammates across research, writing, coding, shared chats, and your connected providers.',
    },
    zh: {
      intro:
        '通用智能体工作区，还是专注编码的智能体？Beans 在聊天中组织命名队友，Claude Code 围绕读代码、编辑文件和运行开发命令构建。',
      other: [
        '本地终端、IDE、桌面，以及网页和云会话',
        'Claude 订阅或 Anthropic Console，部分入口支持第三方提供商',
        '依入口使用订阅或 API',
        '编码会话、并行智能体和开发流程',
      ],
      good: '希望在终端、IDE 或 Claude 应用中以编码为中心工作，可以考虑 Claude Code。希望命名队友跨研究、写作和编码协作，并自由连接提供商，可以考虑 Beans。',
    },
  },
} as const
export type ComparisonSlug = keyof typeof products
export function isComparisonSlug(value: string): value is ComparisonSlug {
  return Object.hasOwn(products, value)
}
export function comparisonHead(lng: Language, slug?: ComparisonSlug) {
  const title = slug
    ? `Beans vs ${products[slug].name}`
    : lng === 'zh'
      ? '比较 Beans 与其他 AI 智能体'
      : 'Compare Beans with other AI agent apps'
  const description = slug
    ? products[slug][lng].intro
    : lng === 'zh'
      ? '比较工作方式、模型连接和费用，选择适合你的 AI 工作区。'
      : 'Compare execution, provider connections, and costs. Find the AI workspace that fits your work.'
  const path = `${lng === 'zh' ? '/zh' : ''}/compare${slug ? '/' + slug : ''}`
  return {
    meta: [
      { title },
      { name: 'description', content: description },
      { property: 'og:title', content: title },
      { property: 'og:description', content: description },
    ],
    links: [
      { rel: 'canonical', href: SITE + path },
      ...(['en', 'zh'] as const).map((other) => ({
        rel: 'alternate',
        hrefLang: other,
        href: SITE + `${other === 'zh' ? '/zh' : ''}/compare${slug ? '/' + slug : ''}`,
      })),
    ],
  }
}
export function ComparisonPage({ slug }: { slug?: ComparisonSlug }) {
  const c = useMarketingCopy()
  const { i18n } = useTranslation()
  const lng = i18n.language === 'zh' ? 'zh' : 'en'
  const zh = lng === 'zh'
  const base = zh ? '/zh/compare' : '/compare'
  const product = slug ? products[slug] : undefined
  const copy = product?.[lng]
  const labels = zh
    ? ['工作在哪里运行', '提供商连接', '费用', '工作方式']
    : ['Where work runs', 'Provider connections', 'Costs', 'Working style']
  const beans = zh
    ? [
        '你拥有的 Mac、Windows 或 Linux 运行设备',
        'ChatGPT、Grok 订阅；Anthropic、DeepSeek、OpenCode API；兼容接口',
        'Beans 免费；提供商费用另付',
        '命名智能体、私聊、群聊、例行任务和加密设备配对',
      ]
    : [
        'Your own Mac, Windows, or Linux Runner',
        'ChatGPT and Grok subscriptions; Anthropic, DeepSeek, OpenCode APIs; compatible endpoints',
        'Beans is free; provider usage is separate',
        'Named bots, DMs, groups, routines, and encrypted device pairing',
      ]
  return (
    <MotionExperience>
      <Nav />
      <main className="comparison-page story-section">
        <Link className="text-link" to={product ? base : zh ? '/zh' : '/'}>
          <ArrowLeft size={17} />
          {product ? c.compareAll : 'Beans'}
        </Link>
        <h1 className="display">{product ? `Beans vs ${product.name}` : c.compareTitle}</h1>
        <p className="comparison-intro">{copy?.intro ?? c.compareBody}</p>
        {product && copy ? (
          <>
            <div className="comparison-table-wrap">
              <table>
                <caption>{zh ? '工作方式比较' : 'How the products work'}</caption>
                <thead>
                  <tr>
                    <th scope="col">{zh ? '比较内容' : 'What matters'}</th>
                    <th scope="col">Beans</th>
                    <th scope="col">{product.name}</th>
                  </tr>
                </thead>
                <tbody>
                  {labels.map((label, index) => (
                    <tr key={label}>
                      <th scope="row">{label}</th>
                      <td>{beans[index]}</td>
                      <td>{copy.other[index]}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <section className="comparison-verdict">
              <h2>{zh ? '哪一个适合你？' : 'Which fits your work?'}</h2>
              <p>{copy.good}</p>
              <Link className="marketing-button" to={downloadPath(lng)}>
                {c.download}
                <ArrowUpRight size={18} />
              </Link>
            </section>
            <div className="comparison-sources">
              <h2>{zh ? '来源与更新' : 'Sources and freshness'}</h2>
              <p>
                {zh
                  ? '依据产品文档，核对日期：2026 年 10 月 7 日。功能和套餐可能变化。'
                  : 'Based on product documentation checked October 7, 2026. Features and plans can change.'}
              </p>
              <a href={product.source}>
                {product.name} — {zh ? '官方产品文档' : 'official product documentation'}
                <ArrowUpRight size={15} />
              </a>
              <a href="https://github.com/bloodf/beans/blob/feat/durindoor-fresh-start-releases/ARCHITECTURE.md">
                Beans — {zh ? '架构与机制' : 'architecture and mechanisms'}
                <ArrowUpRight size={15} />
              </a>
            </div>
          </>
        ) : null}
        <div className="comparison-grid">
          {comparisonNames
            .filter((item) => item.slug !== slug)
            .map((item) => (
              <Link
                to={base === '/zh/compare' ? '/zh/compare/$product' : '/compare/$product'}
                params={{ product: item.slug }}
                key={item.slug}
              >
                <span>
                  Beans <small>vs</small>
                  <br />
                  {item.name}
                </span>
                <ArrowUpRight size={22} />
              </Link>
            ))}
        </div>
      </main>
      <MarketingFooter />
    </MotionExperience>
  )
}
