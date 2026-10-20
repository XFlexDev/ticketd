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
      <div class="transcript"><pre>{{ t.transcript || 'Empty transcript' }}</pre></div>
    </template>
  </div>
</template>
