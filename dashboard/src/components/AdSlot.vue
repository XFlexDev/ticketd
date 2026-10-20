<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useDisplay } from 'vuetify'

// Ads live on ads.kreatix.dev (separate origin) so their scripts can't reach the dashboard or its session.
// They load only after the page is done and idle, and the slot stays collapsed until an ad actually renders.
const { width } = useDisplay()
const w = computed(() => (width.value >= 780 ? 728 : 320))
const h = computed(() => (w.value === 728 ? 90 : 50))
const ready = ref(false)
const filled = ref(false)

function onMsg(e) {
  if (e.origin === 'https://ads.kreatix.dev' && e.data?.kevAd === 'filled') filled.value = true
}
onMounted(() => {
  window.addEventListener('message', onMsg)
  const go = () => (window.requestIdleCallback || (f => setTimeout(f, 1200)))(() => (ready.value = true), { timeout: 3000 })
  document.readyState === 'complete' ? go() : window.addEventListener('load', go, { once: true })
})
onUnmounted(() => window.removeEventListener('message', onMsg))
</script>

<template>
  <div class="ad-slot" :class="{ filled }" :style="{ '--h': h + 'px' }">
    <iframe v-if="ready" :key="w" :src="`https://ads.kreatix.dev/${w}.html`" :width="w" :height="h" title="Advertisement" scrolling="no" sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox" />
  </div>
</template>
