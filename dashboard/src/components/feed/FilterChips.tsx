import type { ActivityKind } from '@/lib/types'
import { KIND_COLORS } from '@/components/KindBadge'
import { cn } from '@/lib/utils'

const KINDS: { id: ActivityKind; label: string }[] = [
  { id: 'opportunity_detected', label: 'Opps' },
  { id: 'dry_run', label: 'Dry-run' },
  { id: 'full_fill', label: 'Fills' },
  { id: 'partial_fill', label: 'Partial' },
  { id: 'no_fill', label: 'No-fill' },
  { id: 'error', label: 'Errors' },
]

interface FilterChipsProps {
  selected: Set<ActivityKind>
  counts: Record<ActivityKind, number>
  onToggle: (k: ActivityKind) => void
  onClear: () => void
}

export function FilterChips({ selected, counts, onToggle, onClear }: FilterChipsProps) {
  const anyActive = selected.size > 0
  return (
    <div className="flex flex-wrap items-center gap-2">
      {KINDS.map((k) => {
        const active = selected.has(k.id)
        const count = counts[k.id] ?? 0
        return (
          <button
            key={k.id}
            type="button"
            onClick={() => onToggle(k.id)}
            className={cn(
              'group cursor-pointer rounded-full border px-3 py-1 font-mono text-[11px] uppercase tracking-wider transition-colors',
              active
                ? 'border-(--color-arb-text)/50 bg-(--color-arb-surface-hi) text-(--color-arb-text)'
                : 'border-(--color-arb-line) bg-(--color-arb-surface)/60 text-(--color-arb-text-dim) hover:border-(--color-arb-text-faint) hover:text-(--color-arb-text)',
            )}
          >
            <span
              className="mr-1.5 inline-block h-1.5 w-1.5 -translate-y-px rounded-full"
              style={{ backgroundColor: KIND_COLORS[k.id] }}
            />
            {k.label}
            <span className="ml-1.5 text-(--color-arb-text-faint)">{count}</span>
          </button>
        )
      })}
      {anyActive && (
        <button
          type="button"
          onClick={onClear}
          className="cursor-pointer rounded-full border border-transparent px-2 py-1 font-mono text-[11px] uppercase tracking-wider text-(--color-arb-text-faint) hover:text-(--color-arb-text)"
        >
          × clear
        </button>
      )}
    </div>
  )
}
