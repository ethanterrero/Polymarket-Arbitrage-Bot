import { useMemo } from 'react'
import { formatDistanceToNowStrict } from 'date-fns'
import { KindBadge } from '@/components/KindBadge'
import { formatPrice, formatUsd, shortHash } from '@/lib/utils'
import { toNumber, type ActivityRow } from '@/lib/types'

interface TickerProps {
  rows: ActivityRow[]
  /** How many recent rows to show. */
  count?: number
}

/**
 * A quiet, non-marquee strip of the most recent events under the top bar.
 * Just the 3-4 newest, flowed inline with thin separators — no animation,
 * no scrolling, no shouting.
 */
export function Ticker({ rows, count = 4 }: TickerProps) {
  const items = useMemo(() => {
    return rows.slice(0, count).map((r) => ({
      id: r.id,
      kind: r.kind,
      market: r.market_question ?? shortHash(r.condition_id),
      ts: r.ts,
      yes: toNumber(r.yes_price),
      no: toNumber(r.no_price),
      profit: toNumber(r.expected_profit),
    }))
  }, [rows, count])

  if (items.length === 0) {
    return (
      <div className="border-b border-(--color-arb-line) bg-(--color-arb-bg)">
        <div className="px-6 py-2.5 text-xs text-(--color-arb-text-faint)">
          Awaiting events…
        </div>
      </div>
    )
  }

  return (
    <div className="border-b border-(--color-arb-line) bg-(--color-arb-bg)">
      <div className="flex items-center gap-4 overflow-x-auto px-6 py-2.5 text-[12px]">
        <span className="shrink-0 text-(--color-arb-text-faint)">Latest</span>
        {items.map((it, i) => (
          <div
            key={it.id}
            className="flex shrink-0 items-center gap-2"
          >
            {i > 0 && <span className="text-(--color-arb-line)">·</span>}
            <KindBadge kind={it.kind} />
            <span className="max-w-[220px] truncate text-(--color-arb-text-dim)">
              {it.market}
            </span>
            {it.yes !== null && (
              <span className="font-mono text-(--color-arb-yes)">Y {formatPrice(it.yes)}</span>
            )}
            {it.no !== null && (
              <span className="font-mono text-(--color-arb-no)">N {formatPrice(it.no)}</span>
            )}
            {it.profit !== null && it.profit > 0 && (
              <span className="font-mono text-(--color-arb-buy)">+{formatUsd(it.profit)}</span>
            )}
            <span className="font-mono text-[11px] text-(--color-arb-text-faint)">
              {formatDistanceToNowStrict(new Date(it.ts), { addSuffix: true })}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}
