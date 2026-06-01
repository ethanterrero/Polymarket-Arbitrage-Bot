import { Check, X } from 'lucide-react'
import { Card, CardHeader } from '@/components/Card'
import { cn, formatInt, formatUsd } from '@/lib/utils'
import { toNumber } from '@/lib/types'
import type { ActivityRow, SnapshotRow } from '@/lib/types'
import type { RealtimeStatus } from '@/hooks/useActivity'

interface DiagnosticsPageProps {
  status: RealtimeStatus
  activity: ActivityRow[]
  snapshots: SnapshotRow[]
}

export function DiagnosticsPage({ status, activity, snapshots }: DiagnosticsPageProps) {
  const url = import.meta.env.VITE_SUPABASE_URL ?? '(missing)'
  const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY ?? ''
  const projectRef = (() => {
    try {
      const u = new URL(url)
      return u.hostname.split('.')[0]
    } catch {
      return '(unknown)'
    }
  })()

  const latest = snapshots[snapshots.length - 1]
  const firstActivity = activity[activity.length - 1]
  const lastActivity = activity[0]

  return (
    <div className="grid grid-cols-1 gap-6 px-6 py-6 lg:grid-cols-2">
      <Card className="gradient-edge">
        <CardHeader
          title="Connection"
          hint="realtime subscription state + Supabase target"
        />
        <dl className="divide-y divide-(--color-arb-line)/60">
          <Row label="Realtime status">
            <span
              className={cn(
                'rounded-md border px-2 py-0.5 font-mono text-[11px] uppercase tracking-wider',
                status === 'live' && 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300',
                status === 'connecting' && 'border-amber-500/40 bg-amber-500/10 text-amber-300',
                status === 'error' && 'border-red-500/40 bg-red-500/10 text-red-300',
              )}
            >
              {status}
            </span>
          </Row>
          <Row label="Supabase URL" mono>
            {url}
          </Row>
          <Row label="Project ref" mono>
            {projectRef}
          </Row>
          <Row label="Anon key present">
            {anonKey ? (
              <span className="inline-flex items-center gap-1 text-emerald-400">
                <Check className="h-3.5 w-3.5" /> yes ({anonKey.length} chars)
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 text-(--color-arb-err)">
                <X className="h-3.5 w-3.5" /> no
              </span>
            )}
          </Row>
          <Row label="Read-only?">
            <span className="text-(--color-arb-text-dim)">
              yes — RLS gates the anon key
            </span>
          </Row>
        </dl>
      </Card>

      <Card className="gradient-edge">
        <CardHeader
          title="Stream counts"
          hint="what's currently in the React state"
        />
        <dl className="divide-y divide-(--color-arb-line)/60">
          <Row label="Activity rows">{formatInt(activity.length)}</Row>
          <Row label="Snapshot rows">{formatInt(snapshots.length)}</Row>
          <Row label="Latest balance" mono>
            {latest ? formatUsd(toNumber(latest.balance)) : '—'}
          </Row>
          <Row label="Latest exposure" mono>
            {latest ? formatUsd(toNumber(latest.total_exposure)) : '—'}
          </Row>
          <Row label="Open positions" mono>
            {latest ? formatInt(latest.open_positions) : '—'}
          </Row>
          <Row label="Oldest event">
            {firstActivity ? new Date(firstActivity.ts).toLocaleString() : '—'}
          </Row>
          <Row label="Newest event">
            {lastActivity ? new Date(lastActivity.ts).toLocaleString() : '—'}
          </Row>
        </dl>
      </Card>

      <Card className="gradient-edge lg:col-span-2">
        <CardHeader title="How this dashboard talks to the bot" />
        <ol className="space-y-2 text-sm text-(--color-arb-text-dim)">
          <li>
            <span className="text-(--color-arb-primary)">1.</span> The bot's{' '}
            <code className="text-(--color-arb-text)">arb-recorder</code> crate POSTs every
            event to the <code className="text-(--color-arb-text)">activity</code> /{' '}
            <code className="text-(--color-arb-text)">snapshots</code> tables using the
            service-role key (env <code className="text-(--color-arb-text)">SUPABASE_SERVICE_KEY</code>).
          </li>
          <li>
            <span className="text-(--color-arb-primary)">2.</span> Supabase Realtime
            broadcasts each INSERT to the{' '}
            <code className="text-(--color-arb-text)">supabase_realtime</code>{' '}
            publication, which both tables subscribe to.
          </li>
          <li>
            <span className="text-(--color-arb-primary)">3.</span> This dashboard
            subscribes via{' '}
            <code className="text-(--color-arb-text)">postgres_changes</code> over the
            anon key, dedups by row id, and renders.
          </li>
        </ol>
      </Card>
    </div>
  )
}

interface RowProps {
  label: string
  mono?: boolean
  children: React.ReactNode
}

function Row({ label, mono, children }: RowProps) {
  return (
    <div className="flex items-center justify-between gap-4 py-2.5">
      <dt className="text-[11px] uppercase tracking-wider text-(--color-arb-text-faint)">
        {label}
      </dt>
      <dd
        className={cn(
          'min-w-0 truncate text-sm text-(--color-arb-text)',
          mono && 'font-mono text-xs',
        )}
      >
        {children}
      </dd>
    </div>
  )
}
