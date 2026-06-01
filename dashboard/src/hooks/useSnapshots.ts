import { useEffect, useRef, useState } from 'react'
import { supabase } from '@/lib/supabase'
import type { SnapshotRow } from '@/lib/types'

/**
 * Subscribes to the snapshots table: pulls the last N rows (chronological) and
 * appends realtime inserts. Used to power the balance / exposure / open
 * positions time-series charts.
 */
export function useSnapshots(limit = 200) {
  const [rows, setRows] = useState<SnapshotRow[]>([])
  const seen = useRef<Set<string>>(new Set())

  useEffect(() => {
    let cancelled = false

    async function seed() {
      const { data, error } = await supabase
        .from('snapshots')
        .select('*')
        .order('ts', { ascending: false })
        .limit(limit)

      if (cancelled) return
      if (error) {
        console.error('initial snapshots select failed', error)
        return
      }
      // We stored newest-first for the limit; reverse to chronological for
      // the chart x-axis.
      const initial = ((data ?? []) as SnapshotRow[]).slice().reverse()
      initial.forEach((r) => seen.current.add(r.id))
      setRows(initial)
    }
    seed()

    const channel = supabase
      .channel('snapshots-feed')
      .on(
        'postgres_changes',
        { event: 'INSERT', schema: 'public', table: 'snapshots' },
        (payload) => {
          const row = payload.new as SnapshotRow
          if (seen.current.has(row.id)) return
          seen.current.add(row.id)
          setRows((prev) => {
            const next = [...prev, row]
            if (next.length > limit) {
              const dropped = next.slice(0, next.length - limit)
              dropped.forEach((d) => seen.current.delete(d.id))
              return next.slice(next.length - limit)
            }
            return next
          })
        },
      )
      .subscribe()

    return () => {
      cancelled = true
      supabase.removeChannel(channel)
    }
  }, [limit])

  return rows
}
