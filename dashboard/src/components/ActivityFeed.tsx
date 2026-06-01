import { useMemo } from 'react'
import { formatDistanceToNowStrict } from 'date-fns'
import { KindBadge } from './KindBadge'
import { Card, CardHeader } from './Card'
import { cn, formatInt, formatPrice, formatUsd, shortHash } from '@/lib/utils'
import { toNumber, type ActivityRow } from '@/lib/types'

interface ActivityFeedProps {
  rows: ActivityRow[]
  /** When true, fade-slide the first row (used for newly arrived realtime events). */
  animateTop?: boolean
}

function relativeTime(ts: string): string {
  const d = Date.parse(ts)
  if (!Number.isFinite(d)) return ''
  const ageMs = Date.now() - d
  if (ageMs < 2000) return 'just now'
  return formatDistanceToNowStrict(new Date(d), { addSuffix: true })
}

function priceCell(yes: number | null, no: number | null) {
  if (yes !== null && no !== null) {
    return (
      <span className="font-mono text-xs">
        <span className="text-emerald-400">Y {formatPrice(yes)}</span>
        <span className="mx-1 text-zinc-600">·</span>
        <span className="text-rose-400">N {formatPrice(no)}</span>
      </span>
    )
  }
  if (yes !== null)
    return <span className="font-mono text-xs text-emerald-400">Y {formatPrice(yes)}</span>
  if (no !== null)
    return <span className="font-mono text-xs text-rose-400">N {formatPrice(no)}</span>
  return <span className="text-xs text-zinc-600">—</span>
}

export function ActivityFeed({ rows, animateTop = true }: ActivityFeedProps) {
  // Memoize the rendered list to avoid re-computing parseFloat / Date.parse on
  // every parent re-render.
  const items = useMemo(() => {
    return rows.map((r) => ({
      r,
      yes: toNumber(r.yes_price),
      no: toNumber(r.no_price),
      size: toNumber(r.size),
      spread: toNumber(r.net_spread),
      profit: toNumber(r.expected_profit),
      cost: toNumber(r.total_cost),
    }))
  }, [rows])

  return (
    <Card className="flex h-full min-h-[480px] flex-col p-0">
      <div className="border-b border-zinc-800 p-5">
        <CardHeader
          title="Live activity"
          hint={`${formatInt(rows.length)} event${rows.length === 1 ? '' : 's'}`}
          right={rows.length > 0 ? relativeTime(rows[0].ts) : ''}
        />
      </div>
      <div className="flex-1 overflow-y-auto">
        {items.length === 0 ? (
          <div className="flex h-full items-center justify-center p-8 text-sm text-zinc-500">
            Waiting for the bot to emit events…
          </div>
        ) : (
          <ul className="divide-y divide-zinc-800/80">
            {items.map(({ r, yes, no, size, spread, profit, cost }, idx) => (
              <li
                key={r.id}
                className={cn(
                  'flex items-start gap-3 px-5 py-3 transition hover:bg-zinc-900/40',
                  animateTop && idx === 0 && 'animate-slide-in',
                )}
              >
                <div className="flex w-16 shrink-0 flex-col gap-1">
                  <KindBadge kind={r.kind} />
                  {r.is_live && (
                    <span className="font-mono text-[9px] uppercase tracking-wider text-emerald-400">
                      live
                    </span>
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-baseline gap-x-2">
                    <span className="truncate text-sm text-zinc-200">
                      {r.market_question ?? shortHash(r.condition_id)}
                    </span>
                    {r.market_question && r.condition_id && (
                      <span className="font-mono text-[10px] text-zinc-600">
                        {shortHash(r.condition_id)}
                      </span>
                    )}
                  </div>
                  <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-zinc-400">
                    {priceCell(yes, no)}
                    {size !== null && (
                      <span className="font-mono">
                        size <span className="text-zinc-200">{formatPrice(size, 2)}</span>
                      </span>
                    )}
                    {spread !== null && (
                      <span className="font-mono">
                        spread <span className="text-zinc-200">{(spread * 100).toFixed(2)}%</span>
                      </span>
                    )}
                    {profit !== null && profit > 0 && (
                      <span className="font-mono text-emerald-400">
                        +{formatUsd(profit)}
                      </span>
                    )}
                    {cost !== null && (
                      <span className="font-mono">
                        cost <span className="text-zinc-200">{formatUsd(cost)}</span>
                      </span>
                    )}
                  </div>
                </div>
                <div className="shrink-0 whitespace-nowrap pl-2 font-mono text-[11px] text-zinc-500">
                  {relativeTime(r.ts)}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  )
}
