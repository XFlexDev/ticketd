import { reactive } from 'vue'

export const session = reactive({ loaded: false, user: null, guilds: [] })

let pending
export function loadSession(force = false) {
  if (pending && !force) return pending
  pending = fetch('/api/me', { credentials: 'same-origin' })
    .then(r => (r.ok ? r.json() : null))
    .catch(() => null)
    .then(d => {
      session.user = d?.user ?? null
      session.guilds = d?.guilds ?? []
      session.loaded = true
      return session
    })
  return pending
}

export async function api(path, opts = {}) {
  const r = await fetch(path, { credentials: 'same-origin', headers: { 'Content-Type': 'application/json' }, ...opts })
  if (r.status === 401) { location.href = '/api/auth/discord'; return new Promise(() => {}) }
  if (!r.ok) throw new Error((await r.text()) || r.statusText)
  return r.headers.get('content-type')?.includes('json') ? r.json() : r.text()
}

export const inviteUrl = 'https://discord.com/oauth2/authorize?client_id=1561997078039887873&scope=bot%20applications.commands&permissions=93200'

export function guildIcon(g, size = 64) {
  return g?.icon ? `https://cdn.discordapp.com/icons/${g.id}/${g.icon}.webp?size=${size}` : null
}
