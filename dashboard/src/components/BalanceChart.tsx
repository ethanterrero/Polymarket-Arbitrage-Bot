import { useMemo } from 'react'
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { Card, CardHeader } from './Card'
import { formatUsd, formatInt } from '@/lib/utils'
import { toNumber, type SnapshotRow } from '@/lib/types'

interface BalanceChartProps {
  snapshots: SnapshotRow[]
}

interface Point {
  ts: number
  label: string
  balance: number | null
  exposure: number | null
  positions: number
}

function formatTime(ts: number): string {
  const d = new Date(ts)
  return d.toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}

export function BalanceChart({ snapshots }: BalanceChartProps) {
  const data = useMemo<Point[]>(() => {
    return snapshots.map((s) => {
      const ts = Date.parse(s.ts)
      return {
        ts,
        label: formatTime(ts),
        balance: toNumber(s.balance),
        exposure: toNumber(s.total_exposure),
        positions: s.open_positions,
      }
    })
  }, [snapshots])

  return (
    <Card className="flex h-full min-h-[320px] flex-col">
      <CardHeader
        title="Balance & exposure"
        hint="USDC over time, from periodic snapshots"
        right={
          data.length > 0
            ? `${formatInt(data.length)} sample${data.length === 1 ? '' : 's'}`
            : ''
        }
      />
      {data.length < 2 ? (
        <div className="flex flex-1 items-center justify-center text-sm text-zinc-500">
          Need at least 2 snapshots to draw a line — the bot emits one every ~30s when telemetry is enabled.
        </div>
      ) : (
        <div className="flex-1">
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={data} margin={{ top: 8, right: 16, left: 0, bottom: 8 }}>
              <defs>
                <linearGradient id="balance-grad" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--color-arb-buy)" stopOpacity={0.4} />
                  <stop offset="100%" stopColor="var(--color-arb-buy)" stopOpacity={0} />
                </linearGradient>
                <linearGradient id="exposure-grad" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--color-arb-info)" stopOpacity={0.35} />
                  <stop offset="100%" stopColor="var(--color-arb-info)" stopOpacity={0} />
                </linearGradient>
              </defs>
              <CartesianGrid stroke="oklch(0.25 0.01 250)" strokeDasharray="3 6" vertical={false} />
              <XAxis
                dataKey="label"
                stroke="oklch(0.5 0.01 250)"
                tick={{ fontSize: 11 }}
                tickLine={false}
                axisLine={false}
                minTickGap={48}
              />
              <YAxis
                stroke="oklch(0.5 0.01 250)"
                tick={{ fontSize: 11 }}
                tickLine={false}
                axisLine={false}
                width={56}
                tickFormatter={(v) => (v >= 1000 ? `$${(v / 1000).toFixed(1)}k` : `$${v}`)}
              />
              <Tooltip
                contentStyle={{
                  backgroundColor: 'oklch(0.18 0.005 250)',
                  border: '1px solid oklch(0.3 0.01 250)',
                  borderRadius: 8,
                  fontSize: 12,
                }}
                labelStyle={{ color: 'oklch(0.7 0.01 250)' }}
                itemStyle={{ color: 'oklch(0.95 0 0)' }}
                formatter={(value, name) => {
                  const n = typeof value === 'number' ? value : Number(value)
                  const series = typeof name === 'string' ? name : String(name)
                  if (series === 'positions') return [formatInt(n), 'open positions']
                  return [formatUsd(n), series]
                }}
              />
              <Area
                type="monotone"
                dataKey="balance"
                stroke="var(--color-arb-buy)"
                strokeWidth={2}
                fill="url(#balance-grad)"
                isAnimationActive={false}
                name="balance"
              />
              <Area
                type="monotone"
                dataKey="exposure"
                stroke="var(--color-arb-info)"
                strokeWidth={2}
                fill="url(#exposure-grad)"
                isAnimationActive={false}
                name="exposure"
              />
            </AreaChart>
          </ResponsiveContainer>
        </div>
      )}
    </Card>
  )
}
