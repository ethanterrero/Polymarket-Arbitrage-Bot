import { useMemo } from 'react'
import { Activity, DollarSign, Layers, TrendingUp, Zap } from 'lucide-react'
import { Card } from './Card'
import { cn, formatInt, formatUsd } from '@/lib/utils'
import type { ActivityRow, SnapshotRow } from '@/lib/types'
import { toNumber } from '@/lib/types'

interface StatsCardsProps {
  activity: ActivityRow[]
  snapshots: SnapshotRow[]
}

interface TileProps {
  label: string
  value: string
  hint?: string
  icon: React.ReactNode
  accent?: string
}

function Tile({ label, value, hint, icon, accent }: TileProps) {
  return (
    <Card className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium uppercase tracking-wider text-zinc-400">
          {label}
        </span>
        <span className={cn('opacity-70', accent)}>{icon}</span>
      </div>
      <div className="font-mono text-2xl font-semibold text-zinc-100 tabular-nums">
        {value}
      </div>
      {hint && <div className="text-xs text-zinc-500">{hint}</div>}
    </Card>
  )
}

export function StatsCards({ activity, snapshots }: StatsCardsProps) {
  const stats = useMemo(() => {
    const oneHourAgo = Date.now() - 60 * 60 * 1000
    let oppsHour = 0
    let fills = 0
    let dryRuns = 0
    let errors = 0
    let totalProfit = 0
    for (const r of activity) {
      const tsMs = Date.parse(r.ts)
      if (r.kind === 'opportunity_detected' && tsMs >= oneHourAgo) oppsHour++
      if (r.kind === 'full_fill' || r.kind === 'partial_fill') {
        fills++
        const ep = toNumber(r.expected_profit)
        if (ep) totalProfit += ep
      }
      if (r.kind === 'dry_run') dryRuns++
      if (r.kind === 'error') errors++
    }
    const latest = snapshots[snapshots.length - 1]
    return {
      balance: latest ? toNumber(latest.balance) : null,
      exposure: latest ? toNumber(latest.total_exposure) : null,
      openPositions: latest ? latest.open_positions : 0,
      oppsHour,
      fills,
      dryRuns,
      errors,
      totalProfit,
    }
  }, [activity, snapshots])

  return (
    <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
      <Tile
        label="Balance"
        value={formatUsd(stats.balance)}
        hint="USDC, latest snapshot"
        icon={<DollarSign className="h-4 w-4" />}
        accent="text-emerald-400"
      />
      <Tile
        label="Exposure"
        value={formatUsd(stats.exposure)}
        hint={`${stats.openPositions} open position${stats.openPositions === 1 ? '' : 's'}`}
        icon={<Layers className="h-4 w-4" />}
        accent="text-sky-400"
      />
      <Tile
        label="Opps · 1h"
        value={formatInt(stats.oppsHour)}
        hint="detected, pre-risk"
        icon={<Zap className="h-4 w-4" />}
        accent="text-(--color-arb-info)"
      />
      <Tile
        label="Fills"
        value={formatInt(stats.fills)}
        hint={`${formatInt(stats.dryRuns)} dry-run`}
        icon={<Activity className="h-4 w-4" />}
        accent="text-(--color-arb-buy)"
      />
      <Tile
        label="Expected P&L"
        value={formatUsd(stats.totalProfit)}
        hint={stats.errors > 0 ? `${stats.errors} error${stats.errors === 1 ? '' : 's'}` : 'sum of filled ops'}
        icon={<TrendingUp className="h-4 w-4" />}
        accent={stats.errors > 0 ? 'text-(--color-arb-err)' : 'text-emerald-400'}
      />
    </div>
  )
}
