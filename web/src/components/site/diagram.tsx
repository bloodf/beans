import { Laptop, Lock, Smartphone } from 'lucide-react'
import { useTranslation } from 'react-i18next'

/// Your computer, the relay, your phone: a pulse of light runs along the connector from one card
/// to the next, and the relay in the middle can only pass it along. The row stacks on a phone.
export function RelayDiagram() {
  const { t } = useTranslation()
  return (
    <div className="relay-illustration motion-island">
    <div role="img" aria-label={t('relay.alt')} className="relay-diagram flex flex-col items-center sm:flex-row sm:items-stretch">
      <Node kind="computer" icon={Laptop} title={t('relay.nodes.computer.title')} body={t('relay.nodes.computer.body')} status={t('relay.states.encrypt')} />
      <Wire />
      <Node kind="relay" icon={Lock} title={t('relay.nodes.relay.title')} body={t('relay.nodes.relay.body')} status={t('relay.states.forward')} accent />
      <Wire second />
      <Node kind="phone" icon={Smartphone} title={t('relay.nodes.phone.title')} body={t('relay.nodes.phone.body')} status={t('relay.states.decrypt')} />
    </div>
    <p className="relay-caption">{t('relay.caption')}</p>
    </div>
  )
}

function Node({
  icon: Icon,
  title,
  body,
  accent = false,
  kind,
  status,
}: {
  icon: typeof Laptop
  title: string
  body: string
  accent?: boolean
  kind: 'computer' | 'relay' | 'phone'
  status: string
}) {
  return (
    <div className={`relay-node relay-node-${kind}`}>
    <div className="relay-node-surface">
      <span
        className={`relay-node-icon mx-auto flex size-11 items-center justify-center rounded-full ${
          accent ? 'bg-linear-to-br from-violet to-cyan text-white' : 'bg-foreground/[0.06] text-foreground'
        }`}
      >
        <Icon className="size-5" strokeWidth={1.75} />
      </span>
      <p className="mt-3 font-semibold">{title}</p>
      <p className="mt-1 text-sm text-muted-foreground">{body}</p>
      <div className="relay-work" aria-hidden="true"><span>{status}</span><span className="relay-work-dots"><i /><i /><i /></span></div>
      <div className="relay-packets" aria-hidden="true"><i /><i /><i /><i /></div>
    </div>
    </div>
  )
}

/// The connector between two cards. The pulse crosses in the first half of the cycle, so the
/// second wire, half a cycle behind, picks it up where the first one left it.
function Wire({ second = false }: { second?: boolean }) {
  return (
    <div
      className="wire relative h-16 w-0.5 flex-none overflow-hidden rounded-full sm:h-0.5 sm:w-auto sm:flex-1 sm:self-center"
      style={{ animationDelay: second ? '2.4s' : '0.8s' }}
    />
  )
}
