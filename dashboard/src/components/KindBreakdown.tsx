import { useMemo } from 'react'
import { Bar, BarChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import { Card, CardHeader } from './Card'
import { KIND_COLORS } from './KindBadge'
import { formatInt } from '@/lib/utils'
import type { ActivityKind, ActivityRow } from '@/lib/types'

interface KindBreakdownProps {
  rows: ActivityRow[]
}

const ORDER: ActivityKind[] = [
  'opportunity_detected',
  'dry_run',
  'full_fill',
  'partial_fill',
  'no_fill',
  'error',
]

const SHORT: Record<ActivityKind, string> = {
  opportunity_detected: 'Opp',
  dry_run: 'Dry',
  full_fill: 'Fill',
  partial_fill: 'Part',
  no_fill: 'NoFill',
  error: 'Err',
}

export function KindBreakdown({ rows }: KindBreakdownProps) {
  const data = useMemo(() => {
    const counts: Record<ActivityKind, number> = {
      opportunity_detected: 0,
      dry_run: 0,
      full_fill: 0,
      partial_fill: 0,
      no_fill: 0,
      error: 0,
    }
    for (const r of rows) counts[r.kind]++
    return ORDER.map((k) => ({
      name: SHORT[k],
      kind: k,
      count: counts[k],
      fill: KIND_COLORS[k],
    }))
  }, [rows])

  return (
    <Card className="flex h-full min-h-[260px] flex-col">
      <CardHeader
        title="Events by kind"
        hint={`${formatInt(rows.length)} total in window`}
      />
      <div className="flex-1">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} margin={{ top: 8, right: 12, left: 0, bottom: 8 }}>
            <XAxis
              dataKey="name"
              stroke="var(--color-arb-text-faint)"
              tick={{ fontSize: 11 }}
              tickLine={false}
              axisLine={false}
            />
            <YAxis
              stroke="var(--color-arb-text-faint)"
              tick={{ fontSize: 11 }}
              tickLine={false}
              axisLine={false}
              width={32}
              allowDecimals={false}
            />
            <Tooltip
              contentStyle={{
                backgroundColor: 'var(--color-arb-surface)',
                border: '1px solid var(--color-arb-line)',
                borderRadius: 8,
                fontSize: 12,
              }}
              labelStyle={{ color: 'var(--color-arb-text-dim)' }}
              itemStyle={{ color: 'var(--color-arb-text)' }}
              cursor={{ fill: 'var(--color-arb-surface-hi)' }}
              formatter={(value) => {
                const n = typeof value === 'number' ? value : Number(value)
                return [formatInt(n), 'events']
              }}
            />
            <Bar dataKey="count" radius={[4, 4, 0, 0]} isAnimationActive={false} />
          </BarChart>
        </ResponsiveContainer>
      </div>
    </Card>
  )
}
