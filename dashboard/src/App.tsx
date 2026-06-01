import { useMemo } from 'react'
import { Header } from '@/components/Header'
import { StatsCards } from '@/components/StatsCards'
import { ActivityFeed } from '@/components/ActivityFeed'
import { BalanceChart } from '@/components/BalanceChart'
import { KindBreakdown } from '@/components/KindBreakdown'
import { useActivity } from '@/hooks/useActivity'
import { useSnapshots } from '@/hooks/useSnapshots'

function App() {
  const { rows: activity, status } = useActivity(200)
  const snapshots = useSnapshots(200)

  // Derive a couple of header flags from the activity stream so we don't have
  // to thread state through extra hooks.
  const { hasLiveTrades, strategyMode } = useMemo(() => {
    let live = false
    let mode: string | null = null
    for (const r of activity) {
      if (r.is_live) live = true
      if (!mode && r.strategy_mode) mode = r.strategy_mode
      if (live && mode) break
    }
    return { hasLiveTrades: live, strategyMode: mode }
  }, [activity])

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-100">
      <Header
        status={status}
        hasLiveTrades={hasLiveTrades}
        strategyMode={strategyMode}
      />
      <main className="mx-auto flex max-w-7xl flex-col gap-6 px-6 py-6">
        <StatsCards activity={activity} snapshots={snapshots} />

        <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2">
            <BalanceChart snapshots={snapshots} />
          </div>
          <div>
            <KindBreakdown rows={activity} />
          </div>
        </div>

        <ActivityFeed rows={activity} />

        <footer className="pb-4 pt-2 text-center font-mono text-[11px] uppercase tracking-wider text-zinc-600">
          polymarket-arb · supabase realtime · {snapshots.length} snapshots · {activity.length} events
        </footer>
      </main>
    </div>
  )
}

export default App
