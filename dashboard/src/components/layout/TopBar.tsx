import type { RealtimeStatus } from '@/hooks/useActivity'
import { cn } from '@/lib/utils'

interface TopBarProps {
  status: RealtimeStatus
  hasLiveTrades: boolean
  strategyMode: string | null
  /** Page title (shown left). */
  title: string
  /** Page subtitle. */
  hint: string
}

const STATUS_TEXT: Record<RealtimeStatus, string> = {
  connecting: 'Connecting',
  live: 'Online',
  error: 'Reconnecting',
}

const STATUS_DOT: Record<RealtimeStatus, string> = {
  connecting: 'bg-(--color-arb-warn)',
  live: 'bg-(--color-arb-primary) animate-pulse-dot',
  error: 'bg-(--color-arb-err)',
}

export function TopBar({ status, hasLiveTrades, strategyMode, title, hint }: TopBarProps) {
  return (
    <header className="sticky top-0 z-20 border-b border-(--color-arb-line) bg-(--color-arb-bg)/85 backdrop-blur">
      <div className="flex items-center justify-between gap-4 px-6 py-5">
        <div>
          <h1 className="text-xl font-semibold tracking-tight text-(--color-arb-text) capitalize">
            {title}
          </h1>
          <p className="mt-0.5 text-[13px] text-(--color-arb-text-faint)">{hint}</p>
        </div>
        <div className="flex items-center gap-2">
          {strategyMode && (
            <span className="hidden rounded-md border border-(--color-arb-line) bg-(--color-arb-surface) px-2.5 py-1 text-[11px] text-(--color-arb-text-dim) sm:inline">
              strat · {strategyMode}
            </span>
          )}
          <span
            className={cn(
              'rounded-md border px-2.5 py-1 text-[11px] font-medium',
              hasLiveTrades
                ? 'border-(--color-arb-primary)/40 bg-(--color-arb-primary)/10 text-(--color-arb-primary)'
                : 'border-(--color-arb-warn)/40 bg-(--color-arb-warn)/10 text-(--color-arb-warn)',
            )}
          >
            {hasLiveTrades ? 'Live' : 'Dry-run'}
          </span>
          <span className="flex items-center gap-2 rounded-md border border-(--color-arb-line) bg-(--color-arb-surface) px-2.5 py-1 text-[11px] text-(--color-arb-text-dim)">
            <span className={cn('h-1.5 w-1.5 rounded-full', STATUS_DOT[status])} />
            {STATUS_TEXT[status]}
          </span>
        </div>
      </div>
    </header>
  )
}
