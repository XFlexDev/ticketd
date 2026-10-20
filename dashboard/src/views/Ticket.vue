<template>
  <div>
    <div class="text-h5 font-weight-bold mb-4">Ticket #{{ $route.params.id }} transcript</div>
    <v-card border flat><v-card-text><pre style="white-space:pre-wrap;font-family:monospace">{{ text }}</pre></v-card-text></v-card>
  </div>
</template>
<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api'
const route = useRoute()
const text = ref('Loading…')
onMounted(async () => { text.value = await api(`/api/tickets/${route.params.id}/transcript`) })
</script>
