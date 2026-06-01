import { Activity, LayoutGrid, ListTree, Settings2 } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

export type Route = 'overview' | 'markets' | 'activity' | 'diagnostics'

interface NavItem {
  id: Route
  label: string
  hint: string
  icon: ReactNode
}

const NAV: NavItem[] = [
  { id: 'overview', label: 'Overview', hint: 'tiles · charts', icon: <LayoutGrid className="h-4 w-4" /> },
  { id: 'markets', label: 'Markets', hint: 'per-market view', icon: <ListTree className="h-4 w-4" /> },
  { id: 'activity', label: 'Activity', hint: 'full feed · filter', icon: <Activity className="h-4 w-4" /> },
  { id: 'diagnostics', label: 'Diagnostics', hint: 'connection · env', icon: <Settings2 className="h-4 w-4" /> },
]

interface SidebarProps {
  route: Route
  onRouteChange: (r: Route) => void
  activityCount: number
  marketCount: number
}

export function Sidebar({ route, onRouteChange, activityCount, marketCount }: SidebarProps) {
  return (
    <aside className="sticky top-0 hidden h-screen w-60 shrink-0 flex-col border-r border-(--color-arb-line) bg-(--color-arb-bg)/90 backdrop-blur md:flex">
      <div className="flex items-center gap-2 border-b border-(--color-arb-line) px-5 py-5">
        <div className="grid h-9 w-9 place-items-center rounded-lg bg-gradient-to-br from-(--color-arb-accent) to-(--color-arb-primary) text-(--color-arb-bg)">
          <span className="font-display text-base font-bold">π</span>
        </div>
        <div className="flex flex-col leading-tight">
          <span
            className="font-display text-[11px] uppercase tracking-[0.28em] text-(--color-arb-text)"
            style={{ fontFamily: 'var(--font-display)' }}
          >
            polymarket
          </span>
          <span className="text-[10px] uppercase tracking-[0.32em] text-(--color-arb-text-faint)">
            arb · v0.1
          </span>
        </div>
      </div>

      <nav className="flex-1 px-3 py-4">
        <ul className="flex flex-col gap-1">
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
                    'group flex w-full cursor-pointer items-center gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors',
                    active
                      ? 'border-(--color-arb-accent)/40 bg-(--color-arb-accent)/15 text-(--color-arb-text)'
                      : 'border-transparent text-(--color-arb-text-dim) hover:border-(--color-arb-line) hover:bg-(--color-arb-surface) hover:text-(--color-arb-text)',
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
                  <span className="flex-1">
                    <span className="block text-sm font-medium">{n.label}</span>
                    <span className="block text-[10px] uppercase tracking-wider text-(--color-arb-text-faint)">
                      {n.hint}
                    </span>
                  </span>
                  {badge !== null && (
                    <span className="rounded-md border border-(--color-arb-line) bg-(--color-arb-surface) px-1.5 py-0.5 font-mono text-[10px] text-(--color-arb-text-dim)">
                      {badge}
                    </span>
                  )}
                </button>
              </li>
            )
          })}
        </ul>
      </nav>

      <div className="border-t border-(--color-arb-line) px-5 py-4 text-[10px] uppercase tracking-wider text-(--color-arb-text-faint)">
        live feed · supabase realtime
      </div>
    </aside>
  )
}
