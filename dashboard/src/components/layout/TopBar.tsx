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
  connecting: 'connecting',
  live: 'online',
  error: 'reconnecting',
}

const STATUS_DOT: Record<RealtimeStatus, string> = {
  connecting: 'bg-amber-400',
  live: 'bg-emerald-400 animate-pulse-dot',
  error: 'bg-red-500',
}

export function TopBar({ status, hasLiveTrades, strategyMode, title, hint }: TopBarProps) {
  return (
    <header className="sticky top-0 z-20 border-b border-(--color-arb-line) bg-(--color-arb-bg)/80 backdrop-blur">
      <div className="flex items-center justify-between gap-4 px-6 py-4">
        <div>
          <h1
            className="font-display text-xl font-semibold tracking-wide text-(--color-arb-text)"
            style={{ fontFamily: 'var(--font-display)' }}
          >
            {title}
          </h1>
          <p className="mt-0.5 text-xs uppercase tracking-wider text-(--color-arb-text-faint)">
            {hint}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {strategyMode && (
            <span className="hidden rounded-md border border-(--color-arb-line) bg-(--color-arb-surface) px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-(--color-arb-text-dim) sm:inline">
              strat · {strategyMode}
            </span>
          )}
          <span
            className={cn(
              'rounded-md border px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider',
              hasLiveTrades
                ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300'
                : 'border-amber-500/40 bg-amber-500/10 text-amber-300',
            )}
          >
            {hasLiveTrades ? 'LIVE' : 'DRY-RUN'}
          </span>
          <span className="flex items-center gap-2 rounded-md border border-(--color-arb-line) bg-(--color-arb-surface) px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-(--color-arb-text-dim)">
            <span className={cn('h-2 w-2 rounded-full', STATUS_DOT[status])} />
            {STATUS_TEXT[status]}
          </span>
        </div>
      </div>
    </header>
  )
}
