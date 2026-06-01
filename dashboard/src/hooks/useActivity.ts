import { useEffect, useRef, useState } from 'react'
import { supabase } from '@/lib/supabase'
import type { ActivityRow } from '@/lib/types'

export type RealtimeStatus = 'connecting' | 'live' | 'error'

/**
 * Subscribes to the activity feed: pulls the most recent N rows, then listens
 * for inserts in realtime and prepends them. We cap the in-memory list so a
 * long-running demo doesn't bloat the DOM.
 */
export function useActivity(limit = 200) {
  const [rows, setRows] = useState<ActivityRow[]>([])
  const [status, setStatus] = useState<RealtimeStatus>('connecting')
  // Track ids we've already seen so an INSERT event that races our initial
  // select can't show up twice.
  const seen = useRef<Set<string>>(new Set())

  useEffect(() => {
    let cancelled = false

    async function seed() {
      const { data, error } = await supabase
        .from('activity')
        .select('*')
        .order('ts', { ascending: false })
        .limit(limit)

      if (cancelled) return
      if (error) {
        console.error('initial activity select failed', error)
        setStatus('error')
        return
      }
      const initial = (data ?? []) as ActivityRow[]
      initial.forEach((r) => seen.current.add(r.id))
      setRows(initial)
    }
    seed()

    const channel = supabase
      .channel('activity-feed')
      .on(
        'postgres_changes',
        { event: 'INSERT', schema: 'public', table: 'activity' },
        (payload) => {
          const row = payload.new as ActivityRow
          if (seen.current.has(row.id)) return
          seen.current.add(row.id)
          setRows((prev) => {
            const next = [row, ...prev]
            if (next.length > limit) {
              const dropped = next.slice(limit)
              dropped.forEach((d) => seen.current.delete(d.id))
              return next.slice(0, limit)
            }
            return next
          })
        },
      )
      .subscribe((s) => {
        if (s === 'SUBSCRIBED') setStatus('live')
        else if (s === 'CHANNEL_ERROR' || s === 'TIMED_OUT') setStatus('error')
      })

    return () => {
      cancelled = true
      supabase.removeChannel(channel)
    }
  }, [limit])

  return { rows, status }
}
