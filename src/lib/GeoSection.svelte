<script>
  // Settings: the geo data files rules point into (geosite:…, geoip:…, ext:…), see docs/geo-api.md.
  import { onMount } from 'svelte';
  import { app, set, connect, loadGeo, geoAddFile, geoAddUrl, geoUpdate, geoSetEnabled, geoRemove, geoFixMissing } from './store.svelte.js';
  import { KIND, SOURCE, host, many, fixable, suppliers } from './geo.js';
  import { bytes, ago } from './format.js';
  import Field from './Field.svelte';
  import Toggle from './Toggle.svelte';
  import Icon from './Icon.svelte';
  import Loader from './Loader.svelte';

  let section = $state();
  let url = $state('');
  let auto = $state(true);
  let urlError = $state('');

  const live = $derived(app.status.state !== 'off');
  const files = $derived(app.geo ?? []);
  const builtins = $derived(files.filter((f) => f.source === 'builtin'));
  const own = $derived(files.filter((f) => f.source !== 'builtin'));
  const anyUrl = $derived(files.some((f) => f.url));

  onMount(loadGeo);

  // the notice about dropped refs opens the settings right here
  $effect(() => {
    if (app.geoFocus && section) {
      app.geoFocus = false;
      section.scrollIntoView({ block: 'start' });
    }
  });

  // ---- refs the last connection had to drop
  const missing = $derived(app.geoMissing);
  const fix = $derived(missing && app.geo ? fixable(missing.refs, app.geo) : []);
  const coverable = $derived.by(() => {
    if (!missing || !app.geo) return [];
    const ids = new Set(fix.map((f) => f.id));
    return missing.refs.filter((r) => suppliers(r, app.geo).some((f) => ids.has(f.id)));
  });
  const lost = $derived(missing ? missing.refs.filter((r) => !coverable.includes(r)) : []);
  const names = (list) => {
    const n = list.map((f) => f.name);
    return n.length > 1 ? `${n.slice(0, -1).join(', ')} и ${n.at(-1)}` : (n[0] ?? '');
  };

  const updating = (f) => !!(app.busy['geo:' + f.id] || (f.url && app.busy['geo:all']));

  function remove(f) {
    app.popup = {
      kind: 'confirm',
      title: `Удалить ${f.name}?`,
      text: 'Правила, которые ссылаются на этот файл, перестанут работать: при подключении Quadra будет их пропускать.',
      ok: 'Удалить',
      action: () => geoRemove(f.id),
    };
  }

  async function addUrl() {
    const u = url.trim();
    if (!/^https?:\/\/\S+$/i.test(u)) {
      urlError = 'Нужна прямая ссылка на файл, она начинается с http:// или https://';
      return;
    }
    urlError = '';
    const err = await geoAddUrl(u, auto);
    if (err === '') url = '';
    else if (err) urlError = `Не получилось добавить: ${err}`;
  }
</script>

<div class="head" bind:this={section}>
  <h3 class="label">Гео-данные</h3>
  {#if anyUrl}
    <button class="link" disabled={!!app.busy['geo:all']} onclick={() => geoUpdate()}>
      {#if app.busy['geo:all']}<Loader cell={2} />Обновляю{:else}<Icon name="refresh" size={13} />Обновить все{/if}
    </button>
  {/if}
</div>
<p class="note">
  В этих файлах лежат списки сайтов и адресов по категориям. Правила маршрутизации ссылаются на них так: <code>geosite:youtube</code>,
  <code>geoip:ru</code>. Нажми на файл, чтобы увидеть его категории.
</p>

{#if missing}
  <div class="missing" role="status">
    <div class="m-head">
      <Icon name="warn" size={14} />
      <b>Пропущено гео-правил: {missing.refs.length}</b>
      <button class="ibtn sm" title="Скрыть" aria-label="Скрыть" onclick={() => (app.geoMissing = null)}><Icon name="close" size={12} /></button>
    </div>
    <p>
      В конфиге сервера{missing.server ? ` «${missing.server}»` : ''} есть правила с категориями, которых у тебя нет.
      Quadra подключилась без них, всё остальное работает как обычно.
    </p>
    <div class="refs">
      {#each missing.refs as r (r)}<code>{r}</code>{/each}
    </div>
    {#if missing.fixed}
      <p>Файлы скачаны. Правила заработают после переподключения.</p>
      {#if live}<div><button class="btn" onclick={connect}><Icon name="refresh" />Переподключить</button></div>{/if}
    {:else if !app.geo}
      <p class="wait"><Loader cell={3} />Смотрю, из каких файлов их можно взять</p>
    {:else if fix.length}
      <p>
        {#if coverable.length < missing.refs.length}Вернуть можно {coverable.length} из {missing.refs.length}.{/if}
        {coverable.length > 1 ? 'Эти категории' : 'Эта категория'} {coverable.length > 1 ? 'могут' : 'может'} быть в {fix.length > 1 ? 'файлах' : 'файле'} {names(fix)},
        сейчас {fix.length > 1 ? 'они выключены или ещё не скачаны' : 'он выключен или ещё не скачан'}.
      </p>
      {#if lost.length}
        <p>
          Для {#if lost.length > 3}остальных ({lost.length}){:else}{#each lost as r, i (r)}<code>{r}</code>{i < lost.length - 1 ? ', ' : ''}{/each}{/if}
          подходящего файла нет. Его придётся найти и добавить ниже.
        </p>
      {/if}
      <div>
        <button class="btn primary" disabled={!!app.busy['geo:fix']} onclick={geoFixMissing}>
          {#if app.busy['geo:fix']}<Loader cell={3} />Скачиваю{:else}<Icon name="download" />Скачать нужные{/if}
        </button>
      </div>
    {:else}
      <p>Ни в одном из твоих файлов таких категорий нет. Найди файл, в котором они есть, и добавь его ниже по ссылке или с компьютера.</p>
    {/if}
  </div>
{/if}

<Field title="Обновлять гео-данные автоматически" hint="Раз в сутки Quadra скачивает свежие версии встроенных файлов и тех, что добавлены по ссылке">
  <Toggle checked={app.settings.geo_auto_update} label="Обновлять гео-данные автоматически" onchange={(v) => set('geo_auto_update', v)} />
</Field>

{#snippet row(f)}
  {@const busy = updating(f)}
  <div class="file" class:off={!f.enabled}>
    <button class="open" disabled={!f.path} title={f.path ? 'Показать категории' : undefined} onclick={() => (app.popup = { kind: 'geo', id: f.id })}>
      <span class="name">
        <b>{f.name}</b>
        <em class="kind">{KIND[f.kind] ?? f.kind}</em>
        {#if f.path}<span class="more"><Icon name="chevron" size={12} /></span>{/if}
      </span>
      {#if f.description}<span class="desc">{f.description}</span>{/if}
      <span class="meta">
        <span title={f.url ?? ''}>{SOURCE[f.source] ?? f.source}{#if f.source === 'url' && f.url}&nbsp;с {host(f.url)}{/if}</span>
        {#if busy}
          <span class="now">скачиваю</span>
        {:else if f.path}
          <span class="mono">{bytes(f.size)}</span>
          {#if f.kind !== 'ruleset'}<span>{many(f.categories, 'категория', 'категории', 'категорий')}</span>{/if}
          <span>{f.source === 'file' ? 'добавлен' : 'обновлён'} {ago(f.updated_at)}</span>
        {:else}
          <span>ещё не скачан</span>
        {/if}
        {#if f.url && f.auto_update && app.settings.geo_auto_update}<span>обновляется сам</span>{/if}
      </span>
      {#if f.error}<span class="err"><Icon name="warn" size={12} /><span>{f.error}</span></span>{/if}
    </button>
    <div class="ctl">
      <Toggle checked={f.enabled} label="Использовать {f.name}" onchange={(v) => geoSetEnabled(f.id, v)} />
      <button class="ibtn" disabled={!f.url} aria-label="Обновить {f.name}"
              title={f.url ? 'Обновить' : 'Файл добавлен с компьютера, скачать его заново неоткуда'} onclick={() => geoUpdate(f.id)}>
        {#if busy}<Loader cell={3} />{:else}<Icon name="refresh" />{/if}
      </button>
      {#if f.source !== 'builtin'}
        <button class="ibtn del" title="Удалить" aria-label="Удалить {f.name}" onclick={() => remove(f)}><Icon name="trash" /></button>
      {:else}
        <span class="gap" aria-hidden="true"></span>
      {/if}
    </div>
  </div>
{/snippet}

<div class="files" aria-busy={!app.geo && !app.geoError}>
  {#if !app.geo && app.geoError}
    <div class="state">
      <p class="err">Не удалось получить список файлов: {app.geoError}</p>
      <button class="btn" onclick={loadGeo}><Icon name="refresh" />Повторить</button>
    </div>
  {:else if !app.geo}
    <div class="state"><Loader cell={4} /><p>Читаю список файлов</p></div>
  {:else}
    {#if !files.length}
      <p class="empty">
        Гео-данных пока нет. Добавь файл по ссылке или с компьютера, и его категории можно будет использовать в правилах.
      </p>
    {:else}
      {#if builtins.length}
        <span class="group">Встроенные</span>
        {#each builtins as f (f.id)}{@render row(f)}{/each}
      {/if}
      <span class="group">Свои</span>
      {#each own as f (f.id)}
        {@render row(f)}
      {:else}
        <p class="empty">
          Своих файлов пока нет. Добавь файл по ссылке или с компьютера, и его категории можно будет использовать в правилах.
        </p>
      {/each}
    {/if}
  {/if}
</div>

<div class="add">
  <b>Добавить файл</b>
  <form class="enter" onsubmit={(e) => { e.preventDefault(); addUrl(); }}>
    <input class="input mono" bind:value={url} spellcheck="false" autocapitalize="off" autocomplete="off" inputmode="url"
           placeholder="https://…/geosite.dat" aria-label="Ссылка на файл" aria-invalid={!!urlError}
           aria-describedby={urlError ? 'geo-url-error' : undefined} disabled={!!app.busy['geo:url']} oninput={() => (urlError = '')} />
    <button class="btn" disabled={!url.trim() || !!app.busy['geo:url']}>
      {#if app.busy['geo:url']}<Loader cell={3} />Скачиваю{:else}<Icon name="download" />Добавить по ссылке{/if}
    </button>
  </form>
  {#if urlError}<p class="url-err" id="geo-url-error" role="alert">{urlError}</p>{/if}
  <div class="foot">
    <label class="auto">
      <Toggle checked={auto} label="Обновлять этот файл автоматически" onchange={(v) => (auto = v)} />
      <span>обновлять автоматически</span>
    </label>
    <span class="grow"></span>
    <button class="btn" disabled={!!app.busy['geo:file']} onclick={geoAddFile}>
      {#if app.busy['geo:file']}<Loader cell={3} />{:else}<Icon name="file" />{/if}Выбрать файл
    </button>
  </div>
  <p class="hint">Подходят geosite и geoip в формате .dat (как у Xray и v2ray) и наборы правил sing-box в .srs или .json.</p>
</div>

<style>
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 26px 0 6px;
    scroll-margin-top: 12px;
  }
  .head h3 {
    margin: 0;
  }
  .link {
    height: 24px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px;
    margin-right: -6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-2);
    border-radius: var(--r);
    transition: color var(--t-fast), background-color var(--t-fast);
  }
  .link:hover {
    color: var(--text);
    background: var(--surface-3);
  }
  .link[disabled] {
    pointer-events: none;
    color: var(--text-3);
  }
  .note {
    margin: 0 0 4px;
    max-width: 620px;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-3);
  }
  code {
    font-size: 11px;
    color: var(--text-2);
    user-select: text;
  }

  .missing {
    max-width: 620px;
    margin: 12px 0 6px;
    padding: 12px 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: color-mix(in srgb, var(--warn) 7%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--warn) 38%, transparent);
    border-radius: var(--r);
    animation: rise var(--t) var(--ease);
  }
  .m-head {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    color: var(--warn);
  }
  .m-head :global(svg) {
    margin-top: 2px;
  }
  .m-head b {
    flex: 1;
    font-weight: 600;
    color: var(--text);
  }
  .m-head .ibtn {
    margin: -4px -6px 0 0;
  }
  .missing p {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-2);
  }
  .missing .wait {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .refs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .refs code {
    padding: 4px 7px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--line-2);
    border-radius: 2px;
  }
  .ibtn.sm {
    width: 24px;
    height: 24px;
  }

  .files {
    display: flex;
    flex-direction: column;
  }
  .group {
    padding: 18px 0 4px;
    font-size: 11.5px;
    color: var(--text-3);
    border-bottom: 1px solid var(--line);
  }
  .file {
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 60px;
    border-bottom: 1px solid var(--line);
  }
  .open {
    flex: 1;
    min-width: 0;
    align-self: stretch;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 3px;
    padding: 9px 10px;
    margin-left: -10px;
    text-align: left;
    border-radius: var(--r);
    transition: background-color var(--t-fast);
  }
  .open:not([disabled]):hover {
    background: var(--surface);
  }
  .open[disabled] {
    cursor: default;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .name b {
    min-width: 0;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind {
    flex: none;
    font: 500 10px/1 var(--mono);
    font-style: normal;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-3);
    padding: 3px 5px 2px;
    border: 1px solid var(--line-2);
    border-radius: 2px;
  }
  .more {
    color: var(--text-3);
    opacity: 0;
    transform: translateX(-3px);
    transition: opacity var(--t-fast), transform var(--t-fast);
  }
  .open:hover .more,
  .open:focus-visible .more {
    opacity: 1;
    transform: none;
  }
  .desc {
    font-size: 11.5px;
    color: var(--text-2);
  }
  /* facts in a row, a thin rule between them; every fact carries the rule on its left, and the
     first one on each line has it pushed out of the clipped box, so no line starts with a rule */
  .meta {
    display: flex;
    flex-wrap: wrap;
    row-gap: 2px;
    margin-left: -8px;
    overflow: hidden;
    font-size: 11.5px;
    color: var(--text-3);
  }
  .meta > span {
    margin-left: -1px;
    padding: 0 8px;
    border-left: 1px solid var(--line-2);
  }
  .meta .now {
    color: var(--accent-dim);
  }
  :global([data-theme='dark']) .meta .now {
    color: var(--accent);
  }
  .err {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--danger);
  }
  .err :global(svg) {
    margin-top: 1px;
  }
  .file.off .name b,
  .file.off .desc {
    color: var(--text-3);
  }
  .ctl {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .ctl > :global(.toggle) {
    margin-right: 8px;
  }
  .gap {
    width: 30px;
  }
  .del:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .state {
    min-height: 120px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--accent);
    border-bottom: 1px solid var(--line);
  }
  .state p {
    margin: 0;
    font-size: 12px;
    color: var(--text-3);
  }
  .state p.err {
    color: var(--danger);
  }
  .empty {
    margin: 0;
    padding: 16px 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-3);
    border-bottom: 1px solid var(--line);
  }

  .add {
    max-width: 620px;
    margin-top: 16px;
    padding: 12px 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--r);
  }
  .add > b {
    font-weight: 600;
  }
  .enter {
    display: flex;
    gap: 8px;
  }
  .enter .input {
    flex: 1;
    min-width: 0;
  }
  .enter .input[aria-invalid='true'] {
    border-color: color-mix(in srgb, var(--danger) 55%, transparent);
  }
  .url-err {
    margin: -2px 0 0;
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--danger);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-2);
    cursor: pointer;
  }
  .grow {
    flex: 1;
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-3);
  }

  @media (max-width: 720px) {
    .file {
      flex-wrap: wrap;
      gap: 0 8px;
      padding-bottom: 8px;
    }
    .ctl {
      margin-left: auto;
    }
    .enter {
      flex-wrap: wrap;
    }
    .enter .input,
    .enter .btn {
      height: 36px;
    }
    .enter .btn {
      flex: 1 1 100%;
    }
  }
</style>
