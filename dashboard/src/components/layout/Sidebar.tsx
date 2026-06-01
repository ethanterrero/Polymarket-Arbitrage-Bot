import { Activity, LayoutGrid, ListTree, Settings2 } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

export type Route = 'overview' | 'markets' | 'activity' | 'diagnostics'

interface NavItem {
  id: Route
  label: string
  icon: ReactNode
}

const NAV: NavItem[] = [
  { id: 'overview', label: 'Overview', icon: <LayoutGrid className="h-4 w-4" /> },
  { id: 'markets', label: 'Markets', icon: <ListTree className="h-4 w-4" /> },
  { id: 'activity', label: 'Activity', icon: <Activity className="h-4 w-4" /> },
  { id: 'diagnostics', label: 'Diagnostics', icon: <Settings2 className="h-4 w-4" /> },
]

interface SidebarProps {
  route: Route
  onRouteChange: (r: Route) => void
  activityCount: number
  marketCount: number
}

export function Sidebar({ route, onRouteChange, activityCount, marketCount }: SidebarProps) {
  return (
    <aside className="sticky top-0 hidden h-screen w-60 shrink-0 flex-col border-r border-(--color-arb-line) bg-(--color-arb-bg) md:flex">
      <div className="flex items-center gap-3 border-b border-(--color-arb-line) px-5 py-5">
        <div className="grid h-8 w-8 place-items-center rounded-md border border-(--color-arb-primary)/40 bg-(--color-arb-primary)/10 text-(--color-arb-primary)">
          <span className="text-sm font-semibold">π</span>
        </div>
        <div className="flex flex-col leading-tight">
          <span className="text-sm font-semibold text-(--color-arb-text)">polymarket-arb</span>
          <span className="text-[11px] text-(--color-arb-text-faint)">live dashboard</span>
        </div>
      </div>

      <nav className="flex-1 px-3 py-4">
        <ul className="flex flex-col gap-0.5">
          {NAV.map((n) => {
            const active = n.id === route
            const badge =
              n.id === 'activity' ? activityCount : n.id === 'markets' ? marketCount : null
            return (
              <li key={n.id}>
                <button
                  type="button"
                  onClick={() => onRouteChange(n.id)}
                  className={cn(
                    'group flex w-full cursor-pointer items-center gap-3 rounded-md px-3 py-2 text-left text-sm transition-colors',
                    active
                      ? 'bg-(--color-arb-surface) text-(--color-arb-text)'
                      : 'text-(--color-arb-text-dim) hover:bg-(--color-arb-surface)/60 hover:text-(--color-arb-text)',
                  )}
                >
                  <span
                    className={cn(
                      'transition-colors',
                      active ? 'text-(--color-arb-primary)' : 'text-(--color-arb-text-faint) group-hover:text-(--color-arb-text-dim)',
                    )}
                  >
                    {n.icon}
                  </span>
                  <span className="flex-1">{n.label}</span>
                  {badge !== null && (
                    <span className="rounded font-mono text-[10px] text-(--color-arb-text-faint) tabular-nums">
                      {badge}
                    </span>
                  )}
                </button>
              </li>
            )
          })}
        </ul>
      </nav>

      <div className="border-t border-(--color-arb-line) px-5 py-4 text-[11px] text-(--color-arb-text-faint)">
        supabase realtime
      </div>
    </aside>
  )
}
