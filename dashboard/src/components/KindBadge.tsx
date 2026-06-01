import type { ActivityKind } from '@/lib/types'
import { cn } from '@/lib/utils'

/**
 * Color + short label for each `kind` value. Kept in one place so the feed,
 * the stats cards, and the chart legend all match.
 */
const STYLES: Record<ActivityKind, { label: string; cls: string }> = {
  opportunity_detected: {
    label: 'OPP',
    cls: 'border-(--color-arb-info)/40 bg-(--color-arb-info)/15 text-(--color-arb-info)',
  },
  dry_run: {
    label: 'DRY',
    cls: 'border-(--color-arb-warn)/40 bg-(--color-arb-warn)/15 text-(--color-arb-warn)',
  },
  full_fill: {
    label: 'FILL',
    cls: 'border-(--color-arb-buy)/50 bg-(--color-arb-buy)/15 text-(--color-arb-buy)',
  },
  partial_fill: {
    label: 'PART',
    cls: 'border-(--color-arb-buy)/40 bg-(--color-arb-buy)/10 text-(--color-arb-buy)',
  },
  no_fill: {
    label: 'NO-FILL',
    cls: 'border-zinc-700 bg-zinc-800/60 text-zinc-400',
  },
  error: {
    label: 'ERR',
    cls: 'border-(--color-arb-err)/40 bg-(--color-arb-err)/15 text-(--color-arb-err)',
  },
}

interface KindBadgeProps {
  kind: ActivityKind
  className?: string
}

export function KindBadge({ kind, className }: KindBadgeProps) {
  const style = STYLES[kind]
  return (
    <span
      className={cn(
        'inline-flex items-center rounded-md border px-2 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-wider',
        style.cls,
        className,
      )}
    >
      {style.label}
    </span>
  )
}

export const KIND_COLORS: Record<ActivityKind, string> = {
  opportunity_detected: 'var(--color-arb-info)',
  dry_run: 'var(--color-arb-warn)',
  full_fill: 'var(--color-arb-buy)',
  partial_fill: 'var(--color-arb-buy)',
  no_fill: 'var(--color-arb-muted)',
  error: 'var(--color-arb-err)',
}
