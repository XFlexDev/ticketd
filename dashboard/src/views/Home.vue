<template>
  <div class="text-center py-16">
    <div class="text-h3 font-weight-bold mb-4">ticketd</div>
    <div class="text-subtitle-1 text-medium-emphasis mb-8" style="max-width:480px;margin:auto">
      Free ticket bot with an AI agent in the loop. Members open tickets, the AI answers instantly,
      your mods take over when needed. Transcripts and summaries included.
    </div>
    <v-btn href="/api/auth/discord" color="primary" size="large" variant="flat" prepend-icon="mdi-discord" class="mr-3">Manage servers</v-btn>
    <v-btn :href="invite" color="secondary" size="large" variant="tonal" prepend-icon="mdi-plus">Add to Discord</v-btn>
  </div>
</template>
<script setup>
const invite = `https://discord.com/oauth2/authorize?scope=bot%20applications.commands&permissions=268435456`
</script>
