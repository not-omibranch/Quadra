<script>
  // The categories inside geo data files, as the body of a popup. "copy": one file, a click
  // copies the ref to a category. "pick": every enabled file, a click adds the ref to the sites
  // in the exclusions or takes it out. Only the visible rows are in the DOM: a geosite file
  // has well over a thousand categories.
  import { untrack } from 'svelte';
  import { app, copy, save, loadGeo, openGeo } from './store.svelte.js';
  import { invoke } from './api.js';
  import { KIND, refPrefix, geoRef, unit, many, num } from './geo.js';
  import Icon from './Icon.svelte';
  import Loader from './Loader.svelte';

  let { mode = 'copy', file = null, onclose } = $props();

  const picking = $derived(mode === 'pick');
  const ROW = $derived(app.platform === 'android' ? 44 : 32);
  const OVERSCAN = 8;

  // the picker offers every file the core will use; a copy browser looks into the one it was opened for
  const files = $derived((app.geo ?? []).filter((f) => f.path && f.enabled));
  let tab = $state(null);
  const current = $derived(
    picking ? (files.find((f) => f.id === tab) ?? files[0] ?? null) : ((app.geo ?? []).find((f) => f.id === file) ?? null),
  );
  const currentId = $derived(current?.id ?? null);

  let tags = $state(null);
  let error = $state('');
  let token = 0;

  async function load(id) {
    const t = ++token;
    tags = null;
    error = '';
    try {
      const list = await invoke('geo_categories', { id, limit: 20000 });
      if (t === token) tags = list ?? [];
    } catch (e) {
      if (t === token) error = typeof e === 'string' ? e : (e?.message ?? 'что-то пошло не так');
    }
  }

  $effect(() => {
    if (!app.geo) loadGeo();
  });

  $effect(() => {
    const id = currentId;
    if (id) untrack(() => load(id));
  });

  let q = $state('');
  // what is typed may be a whole ref copied from somewhere: "geosite:youtube" finds "youtube"
  const needle = $derived(q.trim().toLowerCase().replace(/^ext:[^:]*:/, '').replace(/^(geosite|geoip|ruleset):!?/, ''));
  const shown = $derived.by(() => {
    if (!tags) return [];
    if (!needle) return tags;
    const starts = [];
    const rest = [];
    for (const t of tags) {
      if (t.tag.startsWith(needle)) starts.push(t);
      else if (t.tag.includes(needle)) rest.push(t);
    }
    return [...starts, ...rest];
  });

  let box = $state();
  let scrollTop = $state(0);
  let height = $state(320);
  let active = $state(0);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(Math.min(shown.length, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));
  const slice = $derived(shown.slice(first, last));

  // a new search or another file starts from the top
  $effect(() => {
    void needle;
    void currentId;
    untrack(() => {
      active = 0;
      scrollTop = 0;
      if (box) box.scrollTop = 0;
    });
  });

  const picked = $derived(new Set(app.settings?.bypass_domains ?? []));
  const pickedRefs = $derived((app.settings?.bypass_domains ?? []).filter((d) => /^(geosite|geoip|ext|ruleset):/.test(d)).length);

  function act(t) {
    if (!t || !current) return;
    const ref = geoRef(current, t.tag);
    if (!picking) {
      copy(ref, `Скопировано: ${ref}`);
      return;
    }
    const list = app.settings.bypass_domains;
    app.settings.bypass_domains = picked.has(ref) ? list.filter((d) => d !== ref) : [...list, ref];
    save();
  }

  // the field takes the keyboard as soon as it appears, also when the list arrives late
  const focus = (node) => node.focus();

  function moveTo(i) {
    active = i;
    if (!box) return;
    const top = i * ROW;
    if (top < box.scrollTop) box.scrollTop = top;
    else if (top + ROW > box.scrollTop + height) box.scrollTop = top + ROW - height;
  }

  function onKey(e) {
    const n = shown.length;
    if (!n) return;
    const page = Math.max(1, Math.floor(height / ROW) - 1);
    const step = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page }[e.key];
    if (step) {
      e.preventDefault();
      moveTo(Math.min(n - 1, Math.max(0, active + step)));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      act(shown[active]);
    }
  }
</script>

{#if picking}
  <h3>Категории сайтов и адресов</h3>
  <p>
    {app.settings.bypass_mode === 'only' ? 'Отмеченные категории пойдут через сервер, остальное напрямую.' : 'Отмеченные категории пойдут мимо сервера.'}
    Категория целиком попадает в список сайтов, как обычный домен.
  </p>
  {#if files.length > 1}
    <div class="tabs" role="tablist" aria-label="Файл">
      {#each files as f (f.id)}
        <button role="tab" aria-selected={f.id === currentId} class:on={f.id === currentId} onclick={() => (tab = f.id)}>
          <span>{f.name}</span>
        </button>
      {/each}
    </div>
  {/if}
{:else if current}
  <h3 class="title"><span>{current.name}</span><em class="kind">{KIND[current.kind]}</em></h3>
  <p>
    {#if current.kind === 'ruleset'}
      Набор правил подключается целиком. Нажми на него, чтобы скопировать ссылку для правил маршрутизации.
    {:else}
      Нажми на категорию, чтобы скопировать ссылку на неё. Её можно вставить в список сайтов на экране маршрутизации или в свои правила.
    {/if}
  </p>
{:else}
  <h3>Категории</h3>
{/if}

{#if !app.geo && !app.geoError}
  <div class="state"><Loader cell={5} /></div>
{:else if !app.geo}
  <div class="state">
    <p class="err">Не удалось получить список файлов: {app.geoError}</p>
    <button class="btn" onclick={loadGeo}><Icon name="refresh" />Повторить</button>
  </div>
{:else if !current}
  <div class="state">
    <p>
      {#if picking}
        Нет ни одного включённого файла с гео-данными. Включи встроенные или добавь свои.
      {:else}
        Этот файл ещё не скачан, категорий в нём пока не видно.
      {/if}
    </p>
    {#if picking}<button class="btn" onclick={openGeo}><Icon name="settings" />Открыть гео-данные</button>{/if}
  </div>
{:else}
  {#if current.kind !== 'ruleset'}
    <label class="find">
      <Icon name="search" size={14} />
      <input bind:value={q} data-focus use:focus spellcheck="false" autocapitalize="off" autocomplete="off" placeholder="Поиск категории"
             role="combobox" aria-expanded="true" aria-controls="geo-tags" aria-autocomplete="list"
             aria-activedescendant={shown.length ? `geo-tag-${active}` : undefined} onkeydown={onKey} />
      {#if q}<button class="ibtn sm" title="Очистить" onclick={() => (q = '')}><Icon name="close" size={12} /></button>{/if}
    </label>
  {/if}

  <div class="list" bind:this={box} bind:clientHeight={height} onscroll={() => (scrollTop = box.scrollTop)}
       class:short={current.kind === 'ruleset'}>
    {#if error}
      <div class="state">
        <p class="err">Не удалось прочитать категории: {error}</p>
        <button class="btn" onclick={() => load(current.id)}><Icon name="refresh" />Повторить</button>
      </div>
    {:else if !tags}
      <div class="state"><Loader cell={5} /></div>
    {:else if !tags.length}
      <div class="state"><p>В этом файле нет ни одной категории</p></div>
    {:else if !shown.length}
      <div class="state"><p>Категории «{q.trim()}» здесь нет</p></div>
    {:else}
      <div class="spacer" id="geo-tags" role="listbox" aria-label="Категории {current.name}" aria-multiselectable={picking}
           style="height:{shown.length * ROW}px">
        {#each slice as t, i (t.tag)}
          {@const n = first + i}
          {@const ref = geoRef(current, t.tag)}
          <button class="tag" class:active={n === active} class:on={picking && picked.has(ref)} id="geo-tag-{n}" tabindex={current.kind === 'ruleset' ? 0 : -1}
                  role="option" aria-selected={picking ? picked.has(ref) : n === active}
                  style="transform:translateY({n * ROW}px);height:{ROW}px"
                  onpointermove={() => n !== active && (active = n)} onclick={() => { active = n; act(t); }}>
            {#if picking}<i class="box">{#if picked.has(ref)}<Icon name="check" size={12} />{/if}</i>{/if}
            <span class="ref code"><em>{refPrefix(current)}</em>{t.tag}</span>
            <span class="count mono">{unit(current, t.count)}</span>
            {#if !picking}<span class="copy" aria-hidden="true"><Icon name="copy" size={13} /></span>{/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<div class="row">
  <span class="sum">
    {#if picking}
      Выбрано категорий: {pickedRefs}
    {:else if tags && current?.kind !== 'ruleset'}
      {needle ? `Найдено: ${num(shown.length)} из ${num(tags.length)}` : many(tags.length, 'категория', 'категории', 'категорий')}
    {/if}
  </span>
  <span class="grow"></span>
  {#if picking}
    <button class="btn primary" onclick={onclose}>Готово</button>
  {:else}
    <button class="btn" onclick={onclose}>Закрыть</button>
  {/if}
</div>

<style>
  h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .title span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind {
    flex: none;
    font: 500 10.5px/1 var(--mono);
    font-style: normal;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-3);
    padding: 4px 6px 3px;
    border: 1px solid var(--line-2);
    border-radius: 2px;
  }
  p {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--text-2);
  }
  .tabs {
    flex: none;
    display: flex;
    gap: 2px;
    padding: 2px;
    flex-wrap: wrap;
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r);
  }
  .tabs button {
    flex: none;
    height: 26px;
    padding: 0 11px;
    border-radius: 2px;
    color: var(--text-2);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    transition: background-color var(--t-fast), color var(--t-fast);
  }
  .tabs button:hover {
    color: var(--text);
    background: var(--surface-3);
  }
  .tabs button.on {
    background: var(--accent-soft-2);
    color: var(--accent-dim);
  }
  :global([data-theme='dark']) .tabs button.on {
    color: var(--accent);
  }
  .find {
    flex: none;
    height: 34px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px 0 10px;
    color: var(--text-3);
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r);
    cursor: text;
  }
  .find:focus-within {
    border-color: var(--accent-line);
  }
  .find input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    user-select: text;
  }
  .find input::placeholder {
    color: var(--text-3);
  }
  .ibtn.sm {
    width: 24px;
    height: 24px;
  }
  .list {
    flex: 1 1 380px;
    min-height: 160px;
    overflow-y: auto;
    overflow-x: hidden;
    margin: 0 -8px;
    position: relative;
  }
  .list.short .state {
    min-height: 0;
    padding: 0;
  }
  .list.short {
    flex: 0 0 auto;
    min-height: 0;
    height: 48px;
  }
  .spacer {
    position: relative;
  }
  .tag {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 8px;
    text-align: left;
    border-radius: var(--r);
    will-change: transform;
  }
  .tag.active {
    background: var(--surface-2);
  }
  .ref {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }
  .ref em {
    font-style: normal;
    color: var(--text-3);
  }
  .count {
    flex: none;
    font-size: 11.5px;
    color: var(--text-3);
  }
  .copy {
    flex: none;
    width: 16px;
    color: var(--text-3);
    opacity: 0;
    transition: opacity var(--t-fast);
  }
  .tag.active .copy {
    opacity: 1;
  }
  .box {
    width: 16px;
    height: 16px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent-ink);
    border: 1px solid var(--text-3);
    border-radius: 2px;
    transition: background-color var(--t-fast), border-color var(--t-fast);
  }
  .tag.on .box {
    background: var(--accent);
    border-color: var(--accent);
  }
  .state {
    height: 100%;
    min-height: 140px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 18px 8px;
    color: var(--accent);
    text-align: center;
  }
  .state p {
    max-width: 340px;
    color: var(--text-3);
  }
  .state p.err {
    color: var(--danger);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .grow {
    flex: 1;
  }
  .sum {
    font-size: 12px;
    color: var(--text-2);
  }
  @media (max-width: 720px) {
    .find {
      height: 40px;
    }
    .box {
      width: 20px;
      height: 20px;
    }
  }
</style>
