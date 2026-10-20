<script setup>
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { mdiArrowLeft } from '@mdi/js'
import { api } from '../api'
const route = useRoute()
const router = useRouter()
const t = ref(null)
const error = ref('')
onMounted(async () => {
  try { t.value = await api(`/api/tickets/${route.params.id}`) } catch (e) { error.value = e.message }
})
const lines = computed(() => (t.value?.transcript || '').split('\n').filter(Boolean).map(line => {
  const parts = []
  const re = /\[attachment: (.+?) (kst:([0-9a-f]{64})|https?:\/\/\S+)\]/g
  let last = 0, m
  while ((m = re.exec(line))) {
    if (m.index > last) parts.push({ text: line.slice(last, m.index) })
    const url = m[3] ? `/api/tickets/${route.params.id}/files/${m[3]}` : m[2]
    parts.push({ file: m[1], url, image: /\.(png|jpe?g|gif|webp|bmp)$/i.test(m[1]) })
    last = re.lastIndex
  }
  if (last < line.length) parts.push({ text: line.slice(last) })
  return parts
}))
function back() { history.length > 1 ? router.back() : router.push('/') }
const name = id => (id && t.value?.users?.[id]) || id
const when = s => s ? new Date(s * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : '-'
const rows = computed(() => {
  if (!t.value) return []
  const r = [
    ['Status', t.value.status],
    ['Opened by', name(t.value.user_id)],
    ['Opened', when(t.value.created_at)],
    ['Priority', t.value.priority || 'Not set'],
    ['Claimed by', t.value.claimed_by ? name(t.value.claimed_by) : 'Nobody'],
  ]
  if (t.value.escalated) r.push(['Escalated', 'Yes'])
  if (t.value.status === 'closed') {
    r.push(['Closed', when(t.value.closed_at)])
    r.push(['Closed by', t.value.closed_by ? name(t.value.closed_by) : 'Kev'])
  }
  if (t.value.rating) r.push(['Rating', `${t.value.rating}/5${t.value.rating_comment ? `: ${t.value.rating_comment}` : ''}`])
  return r
})
</script>

<template>
  <div class="workspace narrow">
    <nav class="back-navigation"><v-btn variant="text" :prepend-icon="mdiArrowLeft" @click="back">Back</v-btn></nav>
    <div class="workspace-heading">
      <div style="min-width:0">
        <h1>{{ t?.subject || `Ticket #${route.params.id}` }}</h1>
        <p class="muted small mono id-line">#{{ route.params.id }}</p>
      </div>
    </div>
    <p v-if="error" class="muted">{{ error }}</p>
    <div v-else-if="!t" class="transcript"><div class="skeleton-row" style="border:0;height:40px"><span /></div></div>
    <template v-else>
      <dl class="details ticket-details">
        <template v-for="[k, v] in rows" :key="k"><dt>{{ k }}</dt><dd :class="k === 'Priority' ? ['prio', t.priority || 'none'] : ''">{{ v }}</dd></template>
      </dl>
      <div class="transcript">
        <p v-if="!lines.length" class="muted">Empty transcript</p>
        <div v-for="(l, i) in lines" :key="i" class="t-line">
          <template v-for="(p, j) in l" :key="j">
            <span v-if="p.text">{{ p.text }}</span>
            <a v-else-if="p.image" :href="p.url" target="_blank" rel="noopener" class="t-img"><img :src="p.url" :alt="p.file" loading="lazy"></a>
            <a v-else :href="p.url" target="_blank" rel="noopener" class="t-file">{{ p.file }}</a>
          </template>
        </div>
      </div>
    </template>
  </div>
</template>
