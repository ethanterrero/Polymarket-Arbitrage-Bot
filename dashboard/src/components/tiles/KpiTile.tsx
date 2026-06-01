import type { ReactNode } from 'react'
import { Card } from '@/components/Card'
import { Sparkline } from './Sparkline'
import { cn } from '@/lib/utils'

interface KpiTileProps {
  label: string
  value: string
  hint?: string
  icon: ReactNode
  /** Accent class for the icon. */
  accent?: string
  /** Optional sparkline. */
  spark?: { values: number[]; color?: string }
}

export function KpiTile({ label, value, hint, icon, accent, spark }: KpiTileProps) {
  return (
    <Card className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-(--color-arb-text-dim)">{label}</span>
        <span className={cn('opacity-70', accent)}>{icon}</span>
      </div>
      <div className="flex items-end justify-between gap-3">
        <div className="font-mono text-2xl font-semibold tracking-tight text-(--color-arb-text) tabular-nums">
          {value}
        </div>
        {spark && (
          <Sparkline values={spark.values} color={spark.color ?? 'var(--color-arb-primary)'} />
        )}
      </div>
      {hint && <div className="text-xs text-(--color-arb-text-faint)">{hint}</div>}
    </Card>
  )
}
