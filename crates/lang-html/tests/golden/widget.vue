<template>
  <button class="btn" @click="increment">Count: {{ count }}</button>
</template>

<script lang="ts">
export default {
  data() {
    return { count: 0 };
  },
};
</script>

<style lang="scss" scoped>
$accent: #c33;
.btn { color: $accent; }
</style>
