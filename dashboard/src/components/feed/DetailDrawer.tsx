import { useEffect } from 'react'
import { X } from 'lucide-react'
import { KindBadge } from '@/components/KindBadge'
import { formatPrice, formatUsd, shortHash } from '@/lib/utils'
import { toNumber, type ActivityRow } from '@/lib/types'

interface DetailDrawerProps {
  row: ActivityRow | null
  onClose: () => void
}

function relativeTs(ts: string): string {
  return new Date(ts).toLocaleString()
}

function detailJson(detail: unknown): string {
  try {
    return JSON.stringify(detail, null, 2)
  } catch {
    return String(detail)
  }
}

export function DetailDrawer({ row, onClose }: DetailDrawerProps) {
  // Close on Escape — standard drawer affordance.
  useEffect(() => {
    if (!row) return
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [row, onClose])

  if (!row) return null

  const yes = toNumber(row.yes_price)
  const no = toNumber(row.no_price)
  const size = toNumber(row.size)
  const spread = toNumber(row.net_spread)
  const profit = toNumber(row.expected_profit)
  const cost = toNumber(row.total_cost)

  return (
    <div className="fixed inset-0 z-50">
      <div
        className="absolute inset-0 bg-black/55 backdrop-blur-sm"
        onClick={onClose}
        aria-hidden="true"
      />
      <div className="animate-drawer absolute right-0 top-0 flex h-full w-full max-w-md flex-col border-l border-(--color-arb-line) bg-(--color-arb-surface) shadow-2xl">
        <div className="flex items-start justify-between gap-3 border-b border-(--color-arb-line) p-5">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <KindBadge kind={row.kind} />
              {row.is_live && (
                <span className="text-[11px] font-medium text-(--color-arb-primary)">
                  Live
                </span>
              )}
            </div>
            <h2 className="mt-2 truncate text-base font-semibold text-(--color-arb-text)">
              {row.market_question ?? shortHash(row.condition_id)}
            </h2>
            <p className="mt-1 font-mono text-[11px] text-(--color-arb-text-faint)">
              {relativeTs(row.ts)}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="cursor-pointer rounded-md border border-(--color-arb-line) p-1.5 text-(--color-arb-text-dim) hover:bg-(--color-arb-surface-hi) hover:text-(--color-arb-text)"
            aria-label="close detail"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-5">
          <dl className="grid grid-cols-2 gap-3 text-sm">
            <Field label="Condition ID" mono>
              {row.condition_id ?? '—'}
            </Field>
            <Field label="Strategy">{row.strategy_mode ?? '—'}</Field>
            {yes !== null && (
              <Field label="YES price" mono>
                <span className="text-(--color-arb-yes)">{formatPrice(yes)}</span>
              </Field>
            )}
            {no !== null && (
              <Field label="NO price" mono>
                <span className="text-(--color-arb-no)">{formatPrice(no)}</span>
              </Field>
            )}
            {size !== null && (
              <Field label="Size" mono>
                {formatPrice(size, 2)}
              </Field>
            )}
            {spread !== null && (
              <Field label="Net spread" mono>
                {(spread * 100).toFixed(2)}%
              </Field>
            )}
            {profit !== null && (
              <Field label="Expected profit" mono>
                <span className="text-(--color-arb-buy)">{formatUsd(profit)}</span>
              </Field>
            )}
            {cost !== null && (
              <Field label="Total cost" mono>
                {formatUsd(cost)}
              </Field>
            )}
          </dl>

          {row.detail !== null && row.detail !== undefined && (
            <div className="mt-6">
              <div className="mb-2 flex items-center justify-between">
                <span className="text-xs text-(--color-arb-text-faint)">
                  Raw payload (detail)
                </span>
              </div>
              <pre className="max-h-96 overflow-auto rounded-lg border border-(--color-arb-line) bg-(--color-arb-bg) p-3 font-mono text-[11px] leading-relaxed text-(--color-arb-text-dim)">
                {detailJson(row.detail)}
              </pre>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

interface FieldProps {
  label: string
  mono?: boolean
  children: React.ReactNode
}

function Field({ label, mono, children }: FieldProps) {
  return (
    <div className="rounded-md border border-(--color-arb-line) bg-(--color-arb-bg)/50 p-2.5">
      <dt className="text-[11px] text-(--color-arb-text-faint)">
        {label}
      </dt>
      <dd
        className={
          mono
            ? 'mt-0.5 break-all font-mono text-xs text-(--color-arb-text)'
            : 'mt-0.5 text-sm text-(--color-arb-text)'
        }
      >
        {children}
      </dd>
    </div>
  )
}
