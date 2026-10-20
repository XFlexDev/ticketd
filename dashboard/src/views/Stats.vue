<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: { type: String, required: true } })
const days = ref(30)
const s = ref(null)
const error = ref('')

async function load() {
  error.value = ''
  try { s.value = await api(`/api/guilds/${props.guildId}/stats?days=${days.value}`) } catch (e) { error.value = e.message }
}
onMounted(load)
watch(days, load)

function dur(secs) {
  if (secs == null) return '-'
  const m = Math.round(secs / 60)
  if (m < 1) return `${Math.round(secs)}s`
  if (m < 60) return `${m}m`
  const h = m / 60
  if (h < 48) return `${h.toFixed(h < 10 ? 1 : 0)}h`
  return `${Math.round(h / 24)}d`
}
const series = computed(() => {
  if (!s.value) return []
  const today = Math.floor(Date.now() / 86400000)
  const map = Object.fromEntries(s.value.daily.map(d => [d.day, d]))
  return Array.from({ length: days.value }, (_, i) => {
    const d = today - days.value + 1 + i
    return { day: d, opened: map[d]?.opened || 0, closed: map[d]?.closed || 0 }
  })
})
const peak = computed(() => Math.max(1, ...series.value.map(d => d.opened)))
const ratingCount = computed(() => (s.value?.ratings || []).reduce((a, r) => a + r.count, 0))
const ratingAvg = computed(() => ratingCount.value ? (s.value.ratings.reduce((a, r) => a + r.stars * r.count, 0) / ratingCount.value).toFixed(1) : '-')
const aiRate = computed(() => s.value?.closed ? Math.round((s.value.ai_solved || 0) / s.value.closed * 100) + '%' : '-')
const openTotal = computed(() => (s.value?.open_by_priority || []).reduce((a, r) => a + r.count, 0))
function label(d) { return new Date(d * 86400000).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) }
</script>

<template>
  <p v-if="error" class="muted">{{ error }}</p>
  <div v-else-if="!s" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
  <div v-else class="stats">
    <div class="section-head">
      <p class="muted small">Last {{ days }} days</p>
      <v-btn-toggle v-model="days" mandatory density="compact" variant="outlined" divided>
        <v-btn :value="7">7d</v-btn><v-btn :value="30">30d</v-btn><v-btn :value="90">90d</v-btn>
      </v-btn-toggle>
    </div>

    <div class="kpis">
      <div><span class="kpi">{{ s.opened || 0 }}</span><span class="muted small">Opened</span></div>
      <div><span class="kpi">{{ s.closed || 0 }}</span><span class="muted small">Closed</span></div>
      <div><span class="kpi">{{ dur(s.first_reply_secs) }}</span><span class="muted small">First reply</span></div>
      <div><span class="kpi">{{ dur(s.resolve_secs) }}</span><span class="muted small">Time to close</span></div>
      <div><span class="kpi">{{ aiRate }}</span><span class="muted small">Solved by AI</span></div>
      <div><span class="kpi">{{ ratingAvg }}</span><span class="muted small">Rating ({{ ratingCount }})</span></div>
    </div>

    <section class="agent-section">
      <h3>Tickets per day</h3>
      <div class="bars" role="img" :aria-label="`Tickets opened per day over ${days} days`">
        <div v-for="d in series" :key="d.day" class="bar" :title="`${label(d.day)}: ${d.opened} opened, ${d.closed} closed`">
          <span :class="{ zero: !d.opened }" :style="{ height: (d.opened / peak * 100) + '%' }" />
        </div>
      </div>
      <div class="bar-axis muted small"><span>{{ label(series[0].day) }}</span><span>Today</span></div>
    </section>

    <section class="agent-section two-col stats-split">
      <div>
        <h3>Ratings</h3>
        <div v-for="n in [5, 4, 3, 2, 1]" :key="n" class="rating-row">
          <span class="mono">{{ n }}</span>
          <div class="rating-track"><span :style="{ width: ratingCount ? ((s.ratings.find(r => r.stars === n)?.count || 0) / ratingCount * 100) + '%' : '0' }" /></div>
          <span class="muted small mono">{{ s.ratings.find(r => r.stars === n)?.count || 0 }}</span>
        </div>
      </div>
      <div>
        <h3>Open now ({{ openTotal }})</h3>
        <div v-for="p in ['urgent', 'high', 'medium', 'low', '']" :key="p" class="rating-row">
          <span :class="['prio', p || 'none']">{{ p || 'no priority' }}</span>
          <span class="muted small mono" style="margin-left:auto">{{ s.open_by_priority.find(r => r.priority === p)?.count || 0 }}</span>
        </div>
      </div>
    </section>

    <section class="agent-section">
      <h3>Staff</h3>
      <p v-if="!s.staff.length" class="muted small">Nobody has claimed or closed a ticket yet.</p>
      <div v-else class="row-list">
        <div class="file-row staff-row muted small"><span>Member</span><span>Claimed</span><span>Closed</span></div>
        <div v-for="m in s.staff" :key="m.user_id" class="file-row staff-row">
          <span class="ellipsis">{{ s.users[m.user_id] || m.user_id }}</span>
          <span class="mono">{{ m.claimed }}</span>
          <span class="mono">{{ m.closed }}</span>
        </div>
      </div>
    </section>
  </div>
</template>
