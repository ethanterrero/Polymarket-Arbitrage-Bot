import { useMemo, useState } from 'react'
import { Search } from 'lucide-react'
import { formatDistanceToNowStrict } from 'date-fns'
import { Card, CardHeader } from '@/components/Card'
import { KindBadge } from '@/components/KindBadge'
import { cn, formatInt, formatPrice, formatUsd, shortHash } from '@/lib/utils'
import { groupByMarket } from '@/lib/aggregate'
import type { ActivityRow } from '@/lib/types'

interface MarketsPageProps {
  activity: ActivityRow[]
  onSelectMarket: (conditionId: string) => void
}

export function MarketsPage({ activity, onSelectMarket }: MarketsPageProps) {
  const [query, setQuery] = useState('')
  const markets = useMemo(() => groupByMarket(activity), [activity])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return markets
    return markets.filter(
      (m) =>
        (m.question ?? '').toLowerCase().includes(q) ||
        m.conditionId.toLowerCase().includes(q),
    )
  }, [markets, query])

  if (markets.length === 0) {
    return (
      <div className="flex flex-col gap-6 px-6 py-6">
        <Card className="flex min-h-[240px] items-center justify-center text-center">
          <div>
            <p className="text-sm text-(--color-arb-text-dim)">
              No markets observed yet.
            </p>
            <p className="mt-1 text-[11px] text-(--color-arb-text-faint)">
              Markets appear here once the bot emits an event with a{' '}
              <code className="text-(--color-arb-text)">condition_id</code>.
            </p>
          </div>
        </Card>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6 px-6 py-6">
      <Card className="">
        <CardHeader
          title="Markets"
          hint={`${formatInt(markets.length)} unique condition${markets.length === 1 ? '' : 's'} observed`}
        />
        <div className="relative">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-(--color-arb-text-faint)" />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="search question or condition id…"
            className="w-full rounded-lg border border-(--color-arb-line) bg-(--color-arb-bg) py-2 pl-9 pr-3 text-sm text-(--color-arb-text) placeholder:text-(--color-arb-text-faint) focus:border-(--color-arb-accent)/60 focus:outline-none"
          />
        </div>
      </Card>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
        {filtered.map((m) => (
          <button
            key={m.conditionId}
            type="button"
            onClick={() => onSelectMarket(m.conditionId)}
            className={cn(
              'group relative cursor-pointer overflow-hidden rounded-xl border border-(--color-arb-line) bg-(--color-arb-surface)/70 p-5 text-left transition-colors hover:border-(--color-arb-accent)/40 hover:bg-(--color-arb-surface-hi)/70',
            )}
          >
            <div className="flex items-start justify-between gap-2">
              <KindBadge kind={m.lastKind} />
              <span className="text-[11px] text-(--color-arb-text-faint)">
                {formatDistanceToNowStrict(new Date(m.lastTs), { addSuffix: true })}
              </span>
            </div>
            <h3 className="mt-3 line-clamp-2 text-sm font-medium text-(--color-arb-text)">
              {m.question ?? 'untitled market'}
            </h3>
            <p className="mt-1 font-mono text-[10px] text-(--color-arb-text-faint)">
              {shortHash(m.conditionId, 10, 6)}
            </p>

            <div className="mt-4 grid grid-cols-2 gap-2 text-[11px]">
              <Stat label="Events" value={formatInt(m.eventCount)} />
              <Stat label="Fills" value={formatInt(m.fills)} />
              <Stat label="Opps" value={formatInt(m.opportunities)} />
              <Stat label="Errors" value={formatInt(m.errors)} accent={m.errors > 0 ? 'text-(--color-arb-err)' : undefined} />
            </div>

            {(m.lastYesPrice !== null || m.lastNoPrice !== null) && (
              <div className="mt-3 flex items-center gap-3 font-mono text-xs">
                {m.lastYesPrice !== null && (
                  <span className="text-(--color-arb-yes)">Y {formatPrice(m.lastYesPrice)}</span>
                )}
                {m.lastNoPrice !== null && (
                  <span className="text-(--color-arb-no)">N {formatPrice(m.lastNoPrice)}</span>
                )}
              </div>
            )}

            {m.expectedProfit > 0 && (
              <div className="mt-3 font-mono text-xs text-(--color-arb-buy)">
                + {formatUsd(m.expectedProfit)} expected
              </div>
            )}
          </button>
        ))}
      </div>
    </div>
  )
}

interface StatProps {
  label: string
  value: string
  accent?: string
}

function Stat({ label, value, accent }: StatProps) {
  return (
    <div className="rounded-md border border-(--color-arb-line)/70 bg-(--color-arb-bg)/40 px-2.5 py-1.5">
      <div className="text-[11px] text-(--color-arb-text-faint)">{label}</div>
      <div className={cn('font-mono text-sm text-(--color-arb-text) tabular-nums', accent)}>
        {value}
      </div>
    </div>
  )
}
