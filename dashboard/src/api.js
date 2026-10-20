export async function api(path, opts = {}) {
  const r = await fetch(path, { headers: { 'Content-Type': 'application/json' }, ...opts })
  if (r.status === 401) { location.href = '/api/auth/discord'; return new Promise(() => {}) }
  if (!r.ok) throw new Error(await r.text())
  return r.headers.get('content-type')?.includes('json') ? r.json() : r.text()
}
