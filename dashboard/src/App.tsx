import { useMemo, useState } from 'react'
import { Sidebar, type Route } from '@/components/layout/Sidebar'
import { TopBar } from '@/components/layout/TopBar'
import { Ticker } from '@/components/layout/Ticker'
import { OverviewPage } from '@/components/pages/OverviewPage'
import { MarketsPage } from '@/components/pages/MarketsPage'
import { ActivityPage } from '@/components/pages/ActivityPage'
import { DiagnosticsPage } from '@/components/pages/DiagnosticsPage'
import { DetailDrawer } from '@/components/feed/DetailDrawer'
import { useActivity } from '@/hooks/useActivity'
import { useSnapshots } from '@/hooks/useSnapshots'
import { groupByMarket } from '@/lib/aggregate'
import type { ActivityRow } from '@/lib/types'

const PAGE_META: Record<Route, { title: string; hint: string }> = {
  overview: {
    title: 'overview',
    hint: 'live KPIs · balance curve · recent activity',
  },
  markets: {
    title: 'markets',
    hint: 'one card per observed condition_id',
  },
  activity: {
    title: 'activity',
    hint: 'full event feed · filter by kind + search',
  },
  diagnostics: {
    title: 'diagnostics',
    hint: 'connection state · supabase target · stream counts',
  },
}

function App() {
  const { rows: activity, status } = useActivity(300)
  const snapshots = useSnapshots(200)

  const [route, setRoute] = useState<Route>('overview')
  const [selectedRow, setSelectedRow] = useState<ActivityRow | null>(null)
  // Used to cross-link: click a market card → land on Activity with its id pre-filtered.
  const [activityPrefilter, setActivityPrefilter] = useState<string | null>(null)

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

  const marketCount = useMemo(() => groupByMarket(activity).length, [activity])

  function gotoMarket(conditionId: string) {
    setActivityPrefilter(conditionId)
    setRoute('activity')
  }

  // The Activity page owns its own filter state; we re-mount it via key when we
  // arrive with a prefilter, so the new initial-query takes effect cleanly.
  const activityKey = activityPrefilter ?? 'all'

  return (
    <div className="flex min-h-screen bg-(--color-arb-bg) text-(--color-arb-text)">
      <Sidebar
        route={route}
        onRouteChange={(r) => {
          setRoute(r)
          if (r !== 'activity') setActivityPrefilter(null)
        }}
        activityCount={activity.length}
        marketCount={marketCount}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar
          status={status}
          hasLiveTrades={hasLiveTrades}
          strategyMode={strategyMode}
          title={PAGE_META[route].title}
          hint={PAGE_META[route].hint}
        />
        <Ticker rows={activity} />

        <main className="flex-1">
          {route === 'overview' && (
            <OverviewPage
              activity={activity}
              snapshots={snapshots}
              onSelectRow={setSelectedRow}
            />
          )}
          {route === 'markets' && (
            <MarketsPage activity={activity} onSelectMarket={gotoMarket} />
          )}
          {route === 'activity' && (
            <ActivityPage
              key={activityKey}
              activity={
                activityPrefilter
                  ? activity.filter((r) => r.condition_id === activityPrefilter)
                  : activity
              }
              onSelectRow={setSelectedRow}
            />
          )}
          {route === 'diagnostics' && (
            <DiagnosticsPage status={status} activity={activity} snapshots={snapshots} />
          )}
        </main>

        <footer className="border-t border-(--color-arb-line) px-6 py-3 text-center text-xs text-(--color-arb-text-faint)">
          polymarket-arb · supabase realtime · {snapshots.length} snapshots · {activity.length} events
        </footer>
      </div>

      <DetailDrawer row={selectedRow} onClose={() => setSelectedRow(null)} />
    </div>
  )
}

export default App
