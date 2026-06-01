import { createClient } from '@supabase/supabase-js'

/**
 * The browser-side Supabase client. Uses the publishable / anon key, which is
 * gated by RLS to read-only — never put the service_role key in here.
 *
 * Both values come from .env (loaded by Vite at build time, prefix `VITE_`).
 */

const url = import.meta.env.VITE_SUPABASE_URL
const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY

if (!url || !anonKey) {
  // Throw at module load — better than silently building a broken client and
  // exploding inside a hook later.
  throw new Error(
    'VITE_SUPABASE_URL / VITE_SUPABASE_ANON_KEY missing. Copy dashboard/.env.example to dashboard/.env.',
  )
}

export const supabase = createClient(url, anonKey, {
  realtime: {
    params: {
      // Bump from the default 10 events/sec — a busy scanner can fire many
      // opportunity_detected rows in close succession.
      eventsPerSecond: 50,
    },
  },
})
