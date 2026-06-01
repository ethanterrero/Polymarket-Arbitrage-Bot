/**
 * Row shapes that mirror the Supabase tables defined in
 * `supabase/migrations/20260531_init_dashboard_schema.sql`.
 *
 * Supabase serializes `numeric` columns as JS numbers when the value fits a
 * double, or as strings when precision would be lost. The bot only writes f64,
 * so number is the common case — we still accept string for safety.
 */

export type ActivityKind =
  | 'opportunity_detected'
  | 'dry_run'
  | 'full_fill'
  | 'partial_fill'
  | 'no_fill'
  | 'error'

export type Numeric = number | string | null

export interface ActivityRow {
  id: string
  ts: string
  kind: ActivityKind
  is_live: boolean
  strategy_mode: string | null
  condition_id: string | null
  market_question: string | null
  yes_price: Numeric
  no_price: Numeric
  size: Numeric
  net_spread: Numeric
  expected_profit: Numeric
  total_cost: Numeric
  detail: unknown
}

export interface SnapshotRow {
  id: string
  ts: string
  is_live: boolean
  balance: Numeric
  total_exposure: Numeric
  open_positions: number
}

export function toNumber(v: Numeric): number | null {
  if (v === null || v === undefined) return null
  if (typeof v === 'number') return Number.isFinite(v) ? v : null
  const n = Number(v)
  return Number.isFinite(n) ? n : null
}
