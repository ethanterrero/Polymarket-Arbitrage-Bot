import type { ReactNode } from 'react'
import { Card } from '@/components/Card'
import { Sparkline } from './Sparkline'
import { cn } from '@/lib/utils'

interface KpiTileProps {
  label: string
  value: string
  hint?: string
  icon: ReactNode
  /** Accent class for the icon and (optionally) glow on the value. */
  accent?: string
  /** Glow the value text with the primary or accent color. */
  glow?: 'amber' | 'accent' | 'none'
  /** Optional sparkline. */
  spark?: { values: number[]; color?: string }
}

export function KpiTile({
  label,
  value,
  hint,
  icon,
  accent,
  glow = 'none',
  spark,
}: KpiTileProps) {
  return (
    <Card className="gradient-edge flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-medium uppercase tracking-[0.18em] text-(--color-arb-text-dim)">
          {label}
        </span>
        <span className={cn('opacity-80', accent)}>{icon}</span>
      </div>
      <div className="flex items-end justify-between gap-3">
        <div
          className={cn(
            'font-display text-2xl font-semibold text-(--color-arb-text) tabular-nums',
            glow === 'amber' && 'glow-amber',
            glow === 'accent' && 'glow-accent',
          )}
          style={{ fontFamily: 'var(--font-display)' }}
        >
          {value}
        </div>
        {spark && (
          <Sparkline values={spark.values} color={spark.color ?? 'var(--color-arb-primary)'} />
        )}
      </div>
      {hint && <div className="text-[11px] text-(--color-arb-text-faint)">{hint}</div>}
    </Card>
  )
}
