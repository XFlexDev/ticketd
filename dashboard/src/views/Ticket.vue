<script setup>
import { onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { mdiArrowLeft } from '@mdi/js'
import { api } from '../api'
const route = useRoute()
const router = useRouter()
const text = ref(null)
const error = ref('')
onMounted(async () => {
  try { text.value = await api(`/api/tickets/${route.params.id}/transcript`) } catch (e) { error.value = e.message }
})
function back() { history.length > 1 ? router.back() : router.push('/') }
</script>

<template>
  <div class="workspace narrow">
    <nav class="back-navigation"><v-btn variant="text" :prepend-icon="mdiArrowLeft" @click="back">Back</v-btn></nav>
    <div class="workspace-heading"><h1>Ticket #{{ route.params.id }}</h1></div>
    <p v-if="error" class="muted">{{ error }}</p>
    <div v-else-if="text === null" class="transcript"><div class="skeleton-row" style="border:0;height:40px"><span /></div></div>
    <div v-else class="transcript"><pre>{{ text || 'Empty transcript' }}</pre></div>
  </div>
</template>
