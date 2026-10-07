import { useEffect, useRef, useState } from 'react'
import { ArrowUp, Check, FileText, RotateCcw, Search, Terminal } from 'lucide-react'
import { useMarketingCopy } from './marketing-copy'
import { useSiteMotion } from './motion'

export function BotAvatar({ bot }: { bot: number }) {
  return (
    <span className={`bot-avatar bot-avatar-${bot}`} aria-hidden="true">
      <img src="/brand/beans-mark.svg" alt="" width="24" height="24" />
    </span>
  )
}
const names = ['Chef', 'Scout', 'Builder', 'Chef']
export function WorkExamples() {
  const c = useMarketingCopy()
  const { paused, reduced } = useSiteMotion()
  const [selected, setSelected] = useState(0)
  const [step, setStep] = useState(1)
  const [playing, setPlaying] = useState(true)
  const [visible, setVisible] = useState(false)
  const [pageVisible, setPageVisible] = useState(true)
  const host = useRef<HTMLElement>(null)
  const example = c.scenarios[selected] ?? c.scenarios[0]!
  useEffect(() => {
    const observer = new IntersectionObserver(([entry]) =>
      setVisible(Boolean(entry?.isIntersecting)),
    )
    if (host.current) observer.observe(host.current)
    const visibility = () => setPageVisible(!document.hidden)
    visibility()
    document.addEventListener('visibilitychange', visibility)
    return () => {
      observer.disconnect()
      document.removeEventListener('visibilitychange', visibility)
    }
  }, [])
  useEffect(() => {
    if (reduced && playing) {
      setStep(4)
      setPlaying(false)
      return
    }
    if (!playing || paused || !visible || !pageVisible || step >= 4) return
    const timer = setTimeout(() => setStep((value) => value + 1), 2300)
    return () => clearTimeout(timer)
  }, [playing, paused, reduced, visible, pageVisible, step])
  function choose(index: number) {
    setSelected(index)
    setStep(1)
    setPlaying(true)
  }
  return (
    <section id="turns" ref={host} className="story-section story-examples motion-island">
      <div className="story-heading">
        <h2 className="display">{c.demoTitle}</h2>
        <p>{c.demoBody}</p>
      </div>
      <div className="demo-shell example-workbench">
        <aside className="demo-sidebar">
          <img
            className="demo-logo"
            src="/brand/beans-mark.svg"
            alt="Beans"
            width="34"
            height="34"
          />
          <strong>{c.team}</strong>
          {names.slice(0, 3).map((name, index) => (
            <div className="demo-teammate" key={name}>
              <BotAvatar bot={index} />
              <div>
                <b>{name}</b>
                <small>{['ChatGPT', 'DeepSeek', 'Claude'][index]}</small>
              </div>
              <span className="online-dot" />
            </div>
          ))}
          <div className="demo-sidebar-bottom">
            <span className="online-dot" />
            {c.demo}
          </div>
        </aside>
        <div className="demo-main">
          <div className="demo-toolbar">
            <strong># {c.channel}</strong>
            <span>{c.demo}</span>
          </div>
          <div className="demo-scenarios" aria-label={c.demo}>
            {c.scenarios.map((item, index) => (
              <button
                type="button"
                key={item.name}
                aria-pressed={selected === index}
                onClick={() => choose(index)}
              >
                {item.name}
              </button>
            ))}
          </div>
          <div className="demo-transcript" aria-live="polite" aria-atomic="false">
            <div className="demo-user">
              <span>{c.you}</span>
              <p>{example.prompt}</p>
            </div>
            {example.messages.slice(0, step).map((message, index) => (
              <div className="demo-message" key={`${selected}-${index}`}>
                <BotAvatar bot={index === 3 ? 0 : index} />
                <div>
                  <strong>{names[index]}</strong>
                  <p>{message}</p>
                  {index === 1 && (
                    <span className="demo-tool">
                      <Search size={13} />
                      {c.read}
                    </span>
                  )}
                  {index === 2 && (
                    <span className="demo-tool">
                      <Terminal size={13} />
                      {example.file}
                    </span>
                  )}
                </div>
              </div>
            ))}
          </div>
          <div className="demo-composer">
            <span>{step >= 4 ? c.done : c.working}</span>
            <button
              type="button"
              onClick={() => {
                setStep(1)
                setPlaying(true)
              }}
              aria-label={step >= 4 ? c.replay : c.run}
            >
              {step >= 4 ? <RotateCcw size={18} /> : <ArrowUp size={19} />}
            </button>
          </div>
        </div>
        <aside className="demo-output">
          <div className="output-document">
            <FileText size={28} />
            <h3>{example.file}</h3>
            <p>{c.preview}</p>
            <ul>
              {example.result.map((item, index) => (
                <li key={item}>
                  <Check size={16} className={step > index ? 'check-done' : ''} />
                  {item}
                </li>
              ))}
            </ul>
            <div className={`document-stamp ${step >= 4 ? 'stamp-done' : ''}`}>
              {step >= 4 ? c.done : c.working}
            </div>
          </div>
          <div className="demo-team-stack">
            {[0, 1, 2].map((bot) => (
              <BotAvatar key={bot} bot={bot} />
            ))}
            <span>{c.team}</span>
          </div>
        </aside>
      </div>
      <p className="demo-disclaimer">{c.demoNote}</p>
    </section>
  )
}
