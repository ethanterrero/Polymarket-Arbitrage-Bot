import { cn } from '@/lib/utils'
import type { RealtimeStatus } from '@/hooks/useActivity'

interface HeaderProps {
  status: RealtimeStatus
  /** True if any of the visible activity rows have `is_live=true`. */
  hasLiveTrades: boolean
  /** Most recent observed strategy_mode, for display. */
  strategyMode: string | null
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

export function Header({ status, hasLiveTrades, strategyMode }: HeaderProps) {
  return (
    <header className="border-b border-zinc-800 bg-zinc-950/80 backdrop-blur">
      <div className="mx-auto flex max-w-7xl items-center justify-between px-6 py-4">
        <div className="flex items-baseline gap-3">
          <span className="font-mono text-xs uppercase tracking-[0.3em] text-zinc-500">
            polymarket-arb
          </span>
          <span className="text-lg font-semibold text-zinc-100">
            live dashboard
          </span>
        </div>
        <div className="flex items-center gap-3">
          {strategyMode && (
            <span className="rounded-md border border-zinc-800 bg-zinc-900 px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-zinc-400">
              {strategyMode}
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
          <span className="flex items-center gap-2 rounded-md border border-zinc-800 bg-zinc-900 px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-zinc-400">
            <span className={cn('h-2 w-2 rounded-full', STATUS_DOT[status])} />
            {STATUS_TEXT[status]}
          </span>
        </div>
      </div>
    </header>
  )
}
