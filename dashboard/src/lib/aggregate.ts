/**
 * Pure helpers for deriving cross-cutting views from the raw activity / snapshot
 * streams. Kept out of the components so each page can call into the same
 * aggregations without each one re-implementing the math.
 */

import type { ActivityKind, ActivityRow, SnapshotRow } from './types'
import { toNumber } from './types'

export interface MarketSummary {
  conditionId: string
  question: string | null
  eventCount: number
  lastTs: string
  lastKind: ActivityKind
  fills: number
  opportunities: number
  errors: number
  expectedProfit: number
  lastYesPrice: number | null
  lastNoPrice: number | null
}

/** Group activity rows by `condition_id`, producing one summary per market. */
export function groupByMarket(rows: ActivityRow[]): MarketSummary[] {
  const byId = new Map<string, MarketSummary>()
  for (const r of rows) {
    if (!r.condition_id) continue
    const id = r.condition_id
    let m = byId.get(id)
    if (!m) {
      m = {
        conditionId: id,
        question: r.market_question,
        eventCount: 0,
        lastTs: r.ts,
        lastKind: r.kind,
        fills: 0,
        opportunities: 0,
        errors: 0,
        expectedProfit: 0,
        lastYesPrice: null,
        lastNoPrice: null,
      }
      byId.set(id, m)
    }
    m.eventCount++
    if (r.market_question && !m.question) m.question = r.market_question
    if (r.ts > m.lastTs) {
      m.lastTs = r.ts
      m.lastKind = r.kind
      const y = toNumber(r.yes_price)
      const n = toNumber(r.no_price)
      if (y !== null) m.lastYesPrice = y
      if (n !== null) m.lastNoPrice = n
    }
    if (r.kind === 'full_fill' || r.kind === 'partial_fill') {
      m.fills++
      const ep = toNumber(r.expected_profit)
      if (ep !== null) m.expectedProfit += ep
    }
    if (r.kind === 'opportunity_detected') m.opportunities++
    if (r.kind === 'error') m.errors++
  }
  return [...byId.values()].sort((a, b) => (a.lastTs < b.lastTs ? 1 : -1))
}

/** Build a sparkline series from a numeric field on snapshots (oldest → newest). */
export function snapshotSparkline(
  snapshots: SnapshotRow[],
  field: 'balance' | 'total_exposure',
): number[] {
  return snapshots
    .map((s) => toNumber(s[field]))
    .filter((v): v is number => v !== null)
}

/** Bucketed opportunity-per-minute counts for the last `bucketCount` minutes. */
export function opportunitiesPerMinute(rows: ActivityRow[], bucketCount = 30): number[] {
  const now = Date.now()
  const bucketMs = 60_000
  const buckets = new Array<number>(bucketCount).fill(0)
  for (const r of rows) {
    if (r.kind !== 'opportunity_detected') continue
    const tsMs = Date.parse(r.ts)
    const ageMin = Math.floor((now - tsMs) / bucketMs)
    if (ageMin >= 0 && ageMin < bucketCount) {
      // newest bucket = last index, so flip the index
      buckets[bucketCount - 1 - ageMin]++
    }
  }
  return buckets
}

/** Sparkline of cumulative fill counts in chronological order. */
export function fillCumulative(rows: ActivityRow[]): number[] {
  const chrono = [...rows].sort((a, b) => (a.ts < b.ts ? -1 : 1))
  let n = 0
  return chrono.map((r) => {
    if (r.kind === 'full_fill' || r.kind === 'partial_fill') n++
    return n
  })
}
