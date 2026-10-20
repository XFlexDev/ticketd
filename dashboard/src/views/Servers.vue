<template>
  <div class="text-h5 font-weight-bold mb-6">Your servers</div>
  <v-row>
    <v-col v-for="g in guilds" :key="g.id" cols="12" md="4">
      <v-card :to="`/g/${g.id}`" border flat>
        <v-card-title>{{ g.name || g.id }}</v-card-title>
        <v-card-subtitle>{{ g.id }}</v-card-subtitle>
      </v-card>
    </v-col>
    <v-col v-if="!guilds.length" cols="12">
      <v-alert type="info" variant="tonal">No manageable servers with the bot installed. Invite it first.</v-alert>
    </v-col>
  </v-row>
</template>
<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api'
const emit = defineEmits(['loaded'])
const guilds = ref([])
onMounted(async () => { const d = await api('/api/me'); emit('loaded', d.user); guilds.value = d.guilds })
</script>
