import { useTranslation } from 'react-i18next'
import { useEffect, useRef } from 'react'
import { useMarketingCopy } from './marketing-copy'
import { useSiteMotion } from './motion'

export function BotAvatar({ bot }: { bot: number }) {
  return (
    <span className={`bot-avatar bot-avatar-${bot}`} aria-hidden="true">
      <img src="/brand/beans-mark.svg" alt="" width="24" height="24" />
    </span>
  )
}

export function WorkExamples() {
  const c = useMarketingCopy()
  const { i18n } = useTranslation()
  const { paused, reduced } = useSiteMotion()
  const frame = useRef<HTMLIFrameElement>(null)
  function updateMotion() {
    frame.current?.contentWindow?.postMessage(
      { type: 'beans-preview-motion', paused: paused || reduced },
      window.location.origin,
    )
  }
  useEffect(updateMotion, [paused, reduced])
  return (
    <section id="turns" className="story-section story-examples motion-island">
      <div className="story-heading">
        <h2 className="display">{c.demoTitle}</h2>
        <p>{c.demoBody}</p>
      </div>
      <p className="demo-disclaimer" id="app-preview-note">
        {c.demoNote}
      </p>
      <div
        className="app-preview-shell example-workbench"
        role="region"
        aria-label={c.demo}
        tabIndex={0}
      >
        <iframe
          ref={frame}
          className="app-preview-frame"
          src={`/app-preview/index.html?mock=1&platform=linux&language=${i18n.language === 'zh' ? 'zh' : 'en'}`}
          title={c.demo}
          aria-describedby="app-preview-note"
          loading="lazy"
          sandbox="allow-scripts allow-same-origin"
          onLoad={updateMotion}
        />
      </div>
    </section>
  )
}
