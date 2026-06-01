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
  /** Click handler — drives the detail drawer. */
  onSelect?: (row: ActivityRow) => void
  /** Optional title override (e.g. "Recent activity" on Overview vs "Live feed" on Activity page). */
  title?: string
  /** Optional empty-state copy. */
  emptyHint?: string
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
        <span className="text-(--color-arb-yes)">Y {formatPrice(yes)}</span>
        <span className="mx-1 text-(--color-arb-text-faint)">·</span>
        <span className="text-(--color-arb-no)">N {formatPrice(no)}</span>
      </span>
    )
  }
  if (yes !== null)
    return <span className="font-mono text-xs text-(--color-arb-yes)">Y {formatPrice(yes)}</span>
  if (no !== null)
    return <span className="font-mono text-xs text-(--color-arb-no)">N {formatPrice(no)}</span>
  return <span className="text-xs text-(--color-arb-text-faint)">—</span>
}

export function ActivityFeed({
  rows,
  animateTop = true,
  onSelect,
  title = 'Live activity',
  emptyHint = 'Waiting for the bot to emit events…',
}: ActivityFeedProps) {
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

  const interactive = !!onSelect

  return (
    <Card className="flex h-full min-h-[480px] flex-col p-0">
      <div className="border-b border-(--color-arb-line) p-5">
        <CardHeader
          title={title}
          hint={`${formatInt(rows.length)} event${rows.length === 1 ? '' : 's'}`}
          right={rows.length > 0 ? relativeTime(rows[0].ts) : ''}
        />
      </div>
      <div className="flex-1 overflow-y-auto">
        {items.length === 0 ? (
          <div className="flex h-full items-center justify-center p-8 text-sm text-(--color-arb-text-faint)">
            {emptyHint}
          </div>
        ) : (
          <ul className="divide-y divide-(--color-arb-line)/60">
            {items.map(({ r, yes, no, size, spread, profit, cost }, idx) => {
              const content = (
                <>
                  <div className="flex w-16 shrink-0 flex-col gap-1">
                    <KindBadge kind={r.kind} />
                    {r.is_live && (
                      <span className="text-[10px] font-medium text-(--color-arb-primary)">
                        Live
                      </span>
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-baseline gap-x-2">
                      <span className="truncate text-sm text-(--color-arb-text)">
                        {r.market_question ?? shortHash(r.condition_id)}
                      </span>
                      {r.market_question && r.condition_id && (
                        <span className="font-mono text-[10px] text-(--color-arb-text-faint)">
                          {shortHash(r.condition_id)}
                        </span>
                      )}
                    </div>
                    <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-(--color-arb-text-dim)">
                      {priceCell(yes, no)}
                      {size !== null && (
                        <span className="font-mono">
                          size <span className="text-(--color-arb-text)">{formatPrice(size, 2)}</span>
                        </span>
                      )}
                      {spread !== null && (
                        <span className="font-mono">
                          spread{' '}
                          <span className="text-(--color-arb-text)">{(spread * 100).toFixed(2)}%</span>
                        </span>
                      )}
                      {profit !== null && profit > 0 && (
                        <span className="font-mono text-(--color-arb-buy)">
                          +{formatUsd(profit)}
                        </span>
                      )}
                      {cost !== null && (
                        <span className="font-mono">
                          cost <span className="text-(--color-arb-text)">{formatUsd(cost)}</span>
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="shrink-0 whitespace-nowrap pl-2 font-mono text-[11px] text-(--color-arb-text-faint)">
                    {relativeTime(r.ts)}
                  </div>
                </>
              )

              const liCls = cn(
                'flex w-full items-start gap-3 px-5 py-3 text-left transition-colors',
                interactive && 'cursor-pointer hover:bg-(--color-arb-surface-hi)/60',
                animateTop && idx === 0 && 'animate-slide-in',
              )

              return (
                <li key={r.id}>
                  {interactive ? (
                    <button type="button" className={liCls} onClick={() => onSelect?.(r)}>
                      {content}
                    </button>
                  ) : (
                    <div className={liCls}>{content}</div>
                  )}
                </li>
              )
            })}
          </ul>
        )}
      </div>
    </Card>
  )
}
