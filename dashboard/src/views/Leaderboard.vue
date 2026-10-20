<script setup>
import { onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'

const route = useRoute()
const data = ref(null)
const rows = ref([])
const error = ref('')
const more = ref(true)
const loading = ref(false)
let page = 0

async function load() {
  loading.value = true
  try {
    const r = await fetch(`/api/leaderboard/${route.params.id}?page=${page}`)
    if (!r.ok) throw new Error(r.status === 404 ? "This server doesn't have leveling turned on." : 'Couldn\'t load the leaderboard.')
    const j = await r.json()
    data.value = j
    rows.value.push(...j.rows)
    more.value = j.rows.length === 50
    page++
  } catch (e) { error.value = e.message }
  loading.value = false
}
onMounted(load)
const avatar = r => r.avatar || 'https://cdn.discordapp.com/embed/avatars/0.png'
const icon = g => g?.icon ? `https://cdn.discordapp.com/icons/${route.params.id}/${g.icon}.png?size=128` : null
</script>

<template>
  <div class="lb">
    <header v-if="data" class="lb-head">
      <img v-if="icon(data.guild)" :src="icon(data.guild)" alt="">
      <div><h1>{{ data.guild.name || 'Leaderboard' }}</h1><p class="muted small">Leaderboard</p></div>
    </header>
    <p v-if="error" class="muted">{{ error }}</p>
    <p v-else-if="data && !rows.length" class="muted">Nobody has XP yet.</p>
    <div v-for="(r, i) in rows" :key="r.user_id" class="lb-row">
      <span :class="['lb-place', { top: i < 3 }]">{{ i + 1 }}</span>
      <img :src="avatar(r)" alt="" loading="lazy">
      <div class="lb-main">
        <span class="lb-name">{{ r.name }}</span>
        <div class="lb-bar"><span :style="{ width: `${Math.round((r.into / r.need) * 100)}%` }" /></div>
      </div>
      <div class="lb-level"><b>{{ r.level }}</b><span class="muted small">{{ r.xp.toLocaleString() }} XP</span></div>
    </div>
    <v-btn v-if="more && rows.length" variant="tonal" :loading="loading" style="margin-top:16px" @click="load">Load more</v-btn>
  </div>
</template>

<style scoped>
.lb { max-width: 720px; margin: 0 auto; padding: 40px 20px 80px }
.lb-head { display: flex; gap: 16px; align-items: center; margin: 0 0 28px }
.lb-head img { width: 56px; height: 56px; border-radius: 16px }
.lb-head h1 { font-size: 24px; margin: 0 }
.lb-head p { margin: 2px 0 0 }
.lb-row { display: flex; align-items: center; gap: 14px; padding: 12px 0; border-top: 1px solid rgba(255, 255, 255, .06) }
.lb-place { width: 28px; text-align: center; color: rgba(255, 255, 255, .5); font-variant-numeric: tabular-nums }
.lb-place.top { color: rgb(var(--v-theme-primary)); font-weight: 700 }
.lb-row img { width: 40px; height: 40px; border-radius: 50% }
.lb-main { flex: 1; min-width: 0 }
.lb-name { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500 }
.lb-bar { height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .08); margin-top: 8px; overflow: hidden }
.lb-bar span { display: block; height: 100%; background: rgb(var(--v-theme-primary)); border-radius: 2px; transition: width .6s cubic-bezier(.2, .9, .3, 1.1) }
.lb-level { display: flex; flex-direction: column; align-items: flex-end }
.lb-level b { font-size: 18px }
</style>
