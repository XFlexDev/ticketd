<template>
  <div>
    <div class="text-h5 font-weight-bold mb-6">Server {{ $route.params.id }}</div>
    <v-card class="mb-8" border flat>
      <v-card-title>Settings</v-card-title>
      <v-card-text>
        <v-text-field v-model="s.ticket_category_id" label="Ticket category ID" variant="outlined" density="comfortable" />
        <v-text-field v-model="s.mod_role_id" label="Mod role ID" variant="outlined" density="comfortable" />
        <v-text-field v-model="s.log_channel_id" label="Log channel ID (optional)" variant="outlined" density="comfortable" />
        <v-switch v-model="s.ai_enabled" label="AI auto-answers tickets" color="primary" density="compact" />
      </v-card-text>
      <v-card-actions>
        <v-spacer /><v-btn color="primary" variant="flat" :loading="saving" @click="save">Save</v-btn>
      </v-card-actions>
    </v-card>
    <div class="text-h6 mb-4">Tickets</div>
    <v-table>
      <thead><tr><th>#</th><th>User</th><th>Subject</th><th>Status</th><th></th></tr></thead>
      <tbody>
        <tr v-for="t in tickets" :key="t.id">
          <td>{{ t.id }}</td><td>{{ t.user_id }}</td><td>{{ t.subject }}</td>
          <td><v-chip :color="t.status === 'open' ? 'success' : 'default'" size="small">{{ t.status }}</v-chip></td>
          <td><v-btn :to="`/t/${t.id}`" size="small" variant="text">Transcript</v-btn></td>
        </tr>
        <tr v-if="!tickets.length"><td colspan="5" class="text-medium-emphasis">No tickets yet</td></tr>
      </tbody>
    </v-table>
  </div>
</template>
<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api'
const route = useRoute()
const s = ref({ ticket_category_id: '', mod_role_id: '', log_channel_id: '', ai_enabled: true })
const tickets = ref([])
const saving = ref(false)
onMounted(async () => {
  const d = await api(`/api/guilds/${route.params.id}`)
  if (d.settings) s.value = { ...s.value, ...d.settings }
  const t = await api(`/api/guilds/${route.params.id}/tickets`)
  tickets.value = t.tickets
})
async function save() {
  saving.value = true
  await api(`/api/guilds/${route.params.id}/settings`, { method: 'PUT', body: JSON.stringify(s.value) })
  saving.value = false
}
</script>
