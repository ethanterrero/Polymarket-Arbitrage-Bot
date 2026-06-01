import type { ActivityKind } from '@/lib/types'
import { cn } from '@/lib/utils'

/**
 * Label + color for each `kind` value. Kept in one place so the feed,
 * the stats cards, and the chart legend all match.
 */
const STYLES: Record<ActivityKind, { label: string; cls: string }> = {
  opportunity_detected: {
    label: 'Opp',
    cls: 'border-(--color-arb-info)/40 bg-(--color-arb-info)/10 text-(--color-arb-info)',
  },
  dry_run: {
    label: 'Dry',
    cls: 'border-(--color-arb-warn)/40 bg-(--color-arb-warn)/10 text-(--color-arb-warn)',
  },
  full_fill: {
    label: 'Fill',
    cls: 'border-(--color-arb-buy)/50 bg-(--color-arb-buy)/10 text-(--color-arb-buy)',
  },
  partial_fill: {
    label: 'Part',
    cls: 'border-(--color-arb-buy)/40 bg-(--color-arb-buy)/8 text-(--color-arb-buy)',
  },
  no_fill: {
    label: 'No-fill',
    cls: 'border-(--color-arb-line) bg-(--color-arb-surface) text-(--color-arb-text-dim)',
  },
  error: {
    label: 'Err',
    cls: 'border-(--color-arb-err)/40 bg-(--color-arb-err)/10 text-(--color-arb-err)',
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
        'inline-flex items-center rounded border px-1.5 py-0.5 text-[11px] font-medium',
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
