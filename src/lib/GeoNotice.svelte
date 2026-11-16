<script>
  // Under the connection status: the server's config referred to geo data this device does not
  // have, and the core started without those rules. Never blocks anything; the details and the
  // fix are in Settings, «Гео-данные».
  import { app, openGeo, connect } from './store.svelte.js';
  import Icon from './Icon.svelte';

  const m = $derived(app.geoMissing);
</script>

{#if m}
  <div class="note" class:done={m.fixed} role="status">
    <Icon name={m.fixed ? 'check' : 'warn'} size={14} />
    <p>
      {#if m.fixed}
        Нужные гео-данные скачаны. Пропущенные правила заработают после переподключения.
      {:else}
        В конфиге сервера есть гео-правила, которых у тебя нет: {m.refs.length}. Quadra их пропустила.
      {/if}
      {#if m.fixed}
        {#if app.status.state === 'on'}<button class="more" onclick={connect}>Переподключить</button>{/if}
      {:else}
        <button class="more" onclick={openGeo}>Показать</button>
      {/if}
    </p>
    <button class="x" title="Скрыть" aria-label="Скрыть" onclick={() => (app.geoMissing = null)}><Icon name="close" size={11} /></button>
  </div>
{/if}

<style>
  .note {
    margin: -2px 16px 12px;
    padding: 8px 4px 8px 10px;
    display: flex;
    align-items: flex-start;
    gap: 9px;
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 7%, transparent);
    border: 1px solid color-mix(in srgb, var(--warn) 32%, transparent);
    border-radius: var(--r);
    animation: rise var(--t) var(--ease);
  }
  .note.done {
    color: var(--accent);
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }
  .note > :global(svg) {
    margin-top: 1px;
  }
  p {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--text-2);
  }
  .more {
    font-weight: 600;
    color: var(--text);
    text-decoration: underline;
    text-decoration-color: var(--line-2);
    text-underline-offset: 2px;
    transition: text-decoration-color var(--t-fast);
  }
  .more:hover {
    text-decoration-color: currentColor;
  }
  .x {
    width: 20px;
    height: 20px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-3);
    border-radius: 2px;
  }
  .x:hover {
    color: var(--text);
    background: var(--surface-3);
  }
  :global([data-platform='android']) .note {
    margin: 0 14px 10px 16px;
  }
  :global([data-platform='android']) .x {
    width: 32px;
    height: 32px;
    margin: -6px 0 -6px 0;
  }
</style>
