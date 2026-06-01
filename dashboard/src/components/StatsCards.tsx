import { useMemo } from 'react'
import { Activity, DollarSign, Layers, TrendingUp, Zap } from 'lucide-react'
import { KpiTile } from './tiles/KpiTile'
import { cn, formatInt, formatUsd } from '@/lib/utils'
import { toNumber } from '@/lib/types'
import type { ActivityRow, SnapshotRow } from '@/lib/types'
import {
  fillCumulative,
  opportunitiesPerMinute,
  snapshotSparkline,
} from '@/lib/aggregate'

interface StatsCardsProps {
  activity: ActivityRow[]
  snapshots: SnapshotRow[]
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
      balanceSeries: snapshotSparkline(snapshots, 'balance'),
      exposureSeries: snapshotSparkline(snapshots, 'total_exposure'),
      oppsSeries: opportunitiesPerMinute(activity, 30),
      fillsSeries: fillCumulative(activity),
    }
  }, [activity, snapshots])

  return (
    <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
      <KpiTile
        label="Balance"
        value={formatUsd(stats.balance)}
        hint="USDC · latest snapshot"
        icon={<DollarSign className="h-4 w-4" />}
        accent="text-(--color-arb-primary)"
        glow="amber"
        spark={{ values: stats.balanceSeries, color: 'var(--color-arb-buy)' }}
      />
      <KpiTile
        label="Exposure"
        value={formatUsd(stats.exposure)}
        hint={`${stats.openPositions} open position${stats.openPositions === 1 ? '' : 's'}`}
        icon={<Layers className="h-4 w-4" />}
        accent="text-(--color-arb-info)"
        spark={{ values: stats.exposureSeries, color: 'var(--color-arb-info)' }}
      />
      <KpiTile
        label="Opps · 1h"
        value={formatInt(stats.oppsHour)}
        hint="detected · pre-risk"
        icon={<Zap className="h-4 w-4" />}
        accent="text-(--color-arb-accent)"
        glow="accent"
        spark={{ values: stats.oppsSeries, color: 'var(--color-arb-accent)' }}
      />
      <KpiTile
        label="Fills"
        value={formatInt(stats.fills)}
        hint={`${formatInt(stats.dryRuns)} dry-run`}
        icon={<Activity className="h-4 w-4" />}
        accent="text-(--color-arb-buy)"
        spark={{ values: stats.fillsSeries, color: 'var(--color-arb-buy)' }}
      />
      <KpiTile
        label="Expected P&L"
        value={formatUsd(stats.totalProfit)}
        hint={
          stats.errors > 0 ? `${stats.errors} error${stats.errors === 1 ? '' : 's'}` : 'sum of filled ops'
        }
        icon={<TrendingUp className={cn('h-4 w-4', stats.errors > 0 && 'text-(--color-arb-err)')} />}
        accent={stats.errors > 0 ? 'text-(--color-arb-err)' : 'text-(--color-arb-primary)'}
        glow={stats.errors > 0 ? 'none' : 'amber'}
      />
    </div>
  )
}
