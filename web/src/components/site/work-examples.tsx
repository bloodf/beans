import { useState } from 'react'
import { ArrowRight, Check, MessageSquare } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export function WorkExamples() {
  const { t } = useTranslation()
  const examples = t('examples.items', { returnObjects: true })
  const [selected, setSelected] = useState(0)
  const example = examples[selected] ?? examples[0]
  return (
    <section className="story-section story-examples motion-island">
      <div className="story-heading">
        <h2 className="display">{t('examples.title')}</h2>
        <p className="mt-5 text-lg text-muted-foreground">{t('examples.body')}</p>
      </div>
      <div className="example-choices" aria-label={t('examples.choose')}>
        {examples.map((item, index) => <button type="button" key={item.name} aria-pressed={selected === index} onClick={() => setSelected(index)}>{item.name}<ArrowRight size={18} aria-hidden="true" /></button>)}
      </div>
      {example && <div className="example-workbench" aria-live="polite">
        <div className="example-prompt"><MessageSquare size={26} aria-hidden="true" /><p>{example.prompt}</p></div>
        <ol className="example-flow" key={example.name}>
          {example.steps.map((step, index) => <li key={step} style={{ animationDelay: `${index * 1.6}s` }}><span className="example-step-icon" aria-hidden="true"><Check size={20} /><i /><i /><i /></span><span>{step}</span></li>)}
        </ol>
        <p className="example-caption">{t('examples.caption')}</p>
      </div>}
    </section>
  )
}
