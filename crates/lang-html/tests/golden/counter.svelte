<script>
  let count = 0;
  function increment() {
    count += 1;
  }
</script>

<button on:click={increment}>
  Clicked {count} times
</button>

{#if count > 3}
  <p>Many</p>
{/if}

<style>
  button { color: var(--accent); }
</style>
