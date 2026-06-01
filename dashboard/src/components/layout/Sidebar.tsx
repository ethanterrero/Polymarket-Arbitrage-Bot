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
      <div className="flex items-center gap-3 px-5 py-5">
        <div className="grid h-8 w-8 place-items-center rounded-md bg-(--color-arb-primary) text-white">
          <span className="text-sm font-semibold">π</span>
        </div>
        <div className="flex flex-col leading-tight">
          <span className="text-sm font-semibold text-(--color-arb-text)">polymarket-arb</span>
          <span className="text-[12px] text-(--color-arb-text-faint)">live dashboard</span>
        </div>
      </div>

      <nav className="flex-1 px-2 py-2">
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
                    'group relative flex w-full cursor-pointer items-center gap-3 rounded-lg px-3 py-2 text-left text-[13.5px] transition-colors',
                    active
                      ? 'bg-(--color-arb-surface) text-(--color-arb-text)'
                      : 'text-(--color-arb-text-dim) hover:bg-(--color-arb-surface)/50 hover:text-(--color-arb-text)',
                  )}
                >
                  {active && (
                    <span className="absolute inset-y-1.5 left-0 w-0.5 rounded-full bg-(--color-arb-primary)" />
                  )}
                  <span
                    className={cn(
                      'transition-colors',
                      active
                        ? 'text-(--color-arb-primary)'
                        : 'text-(--color-arb-text-faint) group-hover:text-(--color-arb-text-dim)',
                    )}
                  >
                    {n.icon}
                  </span>
                  <span className="flex-1 font-medium">{n.label}</span>
                  {badge !== null && (
                    <span
                      className={cn(
                        'rounded font-mono text-[11px] tabular-nums',
                        active ? 'text-(--color-arb-text-dim)' : 'text-(--color-arb-text-faint)',
                      )}
                    >
                      {badge}
                    </span>
                  )}
                </button>
              </li>
            )
          })}
        </ul>
      </nav>

      <div className="px-5 py-4 text-[11px] text-(--color-arb-text-faint)">
        supabase realtime
      </div>
    </aside>
  )
}
