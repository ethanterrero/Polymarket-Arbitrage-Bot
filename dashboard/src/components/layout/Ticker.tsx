import { useMemo } from 'react'
import { KIND_COLORS } from '@/components/KindBadge'
import { cn, formatPrice, formatUsd, shortHash } from '@/lib/utils'
import type { ActivityRow } from '@/lib/types'
import { toNumber } from '@/lib/types'

interface TickerProps {
  rows: ActivityRow[]
  /** How many recent rows to show. */
  count?: number
}

/**
 * A horizontally-scrolling ticker of the most recent events, styled like a
 * trading-floor tape. Pure CSS animation (paused on hover for readability),
 * so it costs almost nothing per re-render.
 */
export function Ticker({ rows, count = 14 }: TickerProps) {
  const items = useMemo(() => {
    const slice = rows.slice(0, count)
    return slice.map((r) => {
      const yes = toNumber(r.yes_price)
      const no = toNumber(r.no_price)
      const size = toNumber(r.size)
      const profit = toNumber(r.expected_profit)
      return {
        id: r.id,
        kind: r.kind,
        color: KIND_COLORS[r.kind],
        market: r.market_question ?? shortHash(r.condition_id),
        yes,
        no,
        size,
        profit,
      }
    })
  }, [rows, count])

  if (items.length === 0) {
    return (
      <div className="border-b border-(--color-arb-line) bg-(--color-arb-surface)/40">
        <div className="px-6 py-2 font-mono text-[11px] uppercase tracking-wider text-(--color-arb-text-faint)">
          ··· awaiting events ···
        </div>
      </div>
    )
  }

  // Duplicate the list so the marquee loop stays seamless when it wraps.
  const loop = [...items, ...items]

  return (
    <div
      className="relative overflow-hidden border-b border-(--color-arb-line) bg-(--color-arb-surface)/40"
      aria-label="recent events ticker"
    >
      <div className="pointer-events-none absolute inset-y-0 left-0 z-10 w-12 bg-gradient-to-r from-(--color-arb-bg) to-transparent" />
      <div className="pointer-events-none absolute inset-y-0 right-0 z-10 w-12 bg-gradient-to-l from-(--color-arb-bg) to-transparent" />
      <div className="animate-ticker flex w-max gap-8 py-2 pl-6">
        {loop.map((it, i) => (
          <div
            key={`${it.id}-${i}`}
            className="flex items-center gap-2 whitespace-nowrap font-mono text-[11px] uppercase tracking-wider"
          >
            <span
              className="inline-block h-1.5 w-1.5 rounded-full"
              style={{ backgroundColor: it.color }}
            />
            <span className="text-(--color-arb-text-dim)">{it.kind.replace('_', ' ')}</span>
            <span className="text-(--color-arb-text)">{it.market}</span>
            {it.yes !== null && (
              <span className="text-emerald-400">Y {formatPrice(it.yes)}</span>
            )}
            {it.no !== null && <span className="text-rose-400">N {formatPrice(it.no)}</span>}
            {it.size !== null && (
              <span className="text-(--color-arb-text-faint)">
                · sz {formatPrice(it.size, 2)}
              </span>
            )}
            {it.profit !== null && it.profit > 0 && (
              <span className={cn('text-(--color-arb-buy)')}>+{formatUsd(it.profit)}</span>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}
