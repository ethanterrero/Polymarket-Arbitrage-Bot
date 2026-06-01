import { useMemo, useState } from 'react'
import { Search } from 'lucide-react'
import { Card, CardHeader } from '@/components/Card'
import { ActivityFeed } from '@/components/ActivityFeed'
import { FilterChips } from '@/components/feed/FilterChips'
import type { ActivityKind, ActivityRow } from '@/lib/types'

interface ActivityPageProps {
  activity: ActivityRow[]
  onSelectRow: (row: ActivityRow) => void
}

export function ActivityPage({ activity, onSelectRow }: ActivityPageProps) {
  const [selected, setSelected] = useState<Set<ActivityKind>>(new Set())
  const [query, setQuery] = useState('')

  const counts = useMemo(() => {
    const c: Record<ActivityKind, number> = {
      opportunity_detected: 0,
      dry_run: 0,
      full_fill: 0,
      partial_fill: 0,
      no_fill: 0,
      error: 0,
    }
    for (const r of activity) c[r.kind]++
    return c
  }, [activity])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    return activity.filter((r) => {
      if (selected.size > 0 && !selected.has(r.kind)) return false
      if (q) {
        const market = (r.market_question ?? '').toLowerCase()
        const cond = (r.condition_id ?? '').toLowerCase()
        if (!market.includes(q) && !cond.includes(q)) return false
      }
      return true
    })
  }, [activity, selected, query])

  function toggle(k: ActivityKind) {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(k)) next.delete(k)
      else next.add(k)
      return next
    })
  }

  return (
    <div className="flex flex-col gap-6 px-6 py-6">
      <Card className="">
        <CardHeader title="Filter" hint="combine kind chips + market search" />
        <div className="flex flex-col gap-4">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-(--color-arb-text-faint)" />
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="search market question or condition id…"
              className="w-full rounded-lg border border-(--color-arb-line) bg-(--color-arb-bg) py-2 pl-9 pr-3 text-sm text-(--color-arb-text) placeholder:text-(--color-arb-text-faint) focus:border-(--color-arb-accent)/60 focus:outline-none"
            />
          </div>
          <FilterChips
            selected={selected}
            counts={counts}
            onToggle={toggle}
            onClear={() => setSelected(new Set())}
          />
        </div>
      </Card>

      <ActivityFeed
        rows={filtered}
        title="All activity"
        onSelect={onSelectRow}
        emptyHint={
          query || selected.size > 0
            ? 'No events match the current filter — try clearing chips or search.'
            : 'Waiting for the bot to emit events…'
        }
      />
    </div>
  )
}
