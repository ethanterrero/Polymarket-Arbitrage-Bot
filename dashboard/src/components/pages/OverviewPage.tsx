import { StatsCards } from '@/components/StatsCards'
import { BalanceChart } from '@/components/BalanceChart'
import { KindBreakdown } from '@/components/KindBreakdown'
import { ActivityFeed } from '@/components/ActivityFeed'
import type { ActivityRow, SnapshotRow } from '@/lib/types'

interface OverviewPageProps {
  activity: ActivityRow[]
  snapshots: SnapshotRow[]
  onSelectRow: (row: ActivityRow) => void
}

export function OverviewPage({ activity, snapshots, onSelectRow }: OverviewPageProps) {
  const recent = activity.slice(0, 30)
  return (
    <div className="flex flex-col gap-6 px-6 py-6">
      <StatsCards activity={activity} snapshots={snapshots} />

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2">
          <BalanceChart snapshots={snapshots} />
        </div>
        <KindBreakdown rows={activity} />
      </div>

      <ActivityFeed
        rows={recent}
        title="Recent activity"
        onSelect={onSelectRow}
        emptyHint="Waiting for the bot to emit events… (toggle telemetry.enabled in config)"
      />
    </div>
  )
}
