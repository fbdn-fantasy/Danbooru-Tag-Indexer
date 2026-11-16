<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import WikiTree from "./WikiTree.svelte";
  import RichText from "./RichText.svelte";
  import {
    loadsWiki,
    mergePreview,
    type WikiEntry,
    type WikiPage,
    type TreeActions,
  } from "./navigation";
  let {
    entry,
    actions,
    stars,
    trail,
    ancestors,
    openSections,
    command,
  }: {
    entry: WikiEntry;
    actions: TreeActions;
    stars: Set<string>;
    trail: WikiEntry[];
    ancestors: string[];
    openSections: boolean;
    command: { serial: number; open: boolean };
  } = $props();
  let opened = $state(untrack(() => openSections && entry.kind === "section"));
  let page = $state<WikiPage | null>(null);
  let pending = $state(false),
    error = $state(""),
    alive = true;
  let serial = untrack(() => command.serial);
  const wiki = $derived(loadsWiki(entry));
  const cycle = $derived(wiki && ancestors.includes(entry.name));
  const children = $derived(
    page ? mergePreview(page.entries, entry.entries) : entry.entries,
  );
  const description = $derived(page?.description || entry.description);
  $effect(() => {
    if (command.serial !== serial) {
      serial = command.serial;
      if (!command.open) opened = false;
      else if (!wiki || page) opened = true;
    }
  });
  onDestroy(() => {
    alive = false;
  });
  async function toggle() {
    opened = !opened;
    if (!opened || !wiki || page || cycle || pending) return;
    pending = true;
    error = "";
    try {
      const loaded = await actions.load(entry.name);
      if (alive) page = loaded;
    } catch (e) {
      if (alive) error = String(e);
    } finally {
      if (alive) pending = false;
    }
  }
  async function action(fn: () => Promise<void>) {
    try {
      await fn();
    } catch (e) {
      error = String(e);
    }
  }
</script>

<li class="wiki-branch" data-name={entry.name}>
  <div class="wiki-row">
    <button
      class="wiki-arrow"
      aria-label={`${opened ? "Collapse" : "Expand"} ${entry.display}`}
      aria-expanded={opened}
      onclick={() => void toggle()}>{opened ? "⌄" : cycle ? "↩" : "›"}</button
    >
    <button
      class:wiki-heading={entry.kind === "section"}
      class="wiki-label"
      aria-expanded={opened}
      onclick={() => void toggle()}>{entry.display}</button
    >
    {#if entry.kind === "tag"}
      <button
        class="wiki-copy"
        aria-label={`Copy ${entry.display}`}
        onclick={() => void action(() => actions.copy(entry.name))}>Copy</button
      >
      <button
        class="wiki-star"
        class:starred={stars.has(entry.name)}
        aria-label={`${stars.has(entry.name) ? "Unstar" : "Star"} ${entry.display}`}
        aria-pressed={stars.has(entry.name)}
        onclick={() => void action(() => actions.star(entry.name, trail))}
        >{stars.has(entry.name) ? "★" : "☆"}</button
      >
    {/if}
  </div>
  {#if entry.note}<div class="wiki-note">{entry.note}</div>{/if}
  {#if opened}<div class="wiki-body">
      {#if cycle}<p class="muted">This page is already expanded above.</p>
      {:else if pending}<p class="muted">Loading wiki…</p>
      {:else if error}<p role="alert">{error}</p>
        <button
          onclick={() => {
            opened = false;
            void toggle();
          }}>Retry</button
        >
      {:else}
        {#if description}<RichText
            text={description}
            onOpen={actions.open}
          />{/if}
        <WikiTree
          entries={children}
          {actions}
          {stars}
          trail={[...trail, entry]}
          ancestors={[...ancestors, entry.name]}
          openSections={!!page || openSections}
          {command}
        />
        {#if !description && !children.length}<p class="muted">
            No listed contents in this indexed wiki.
          </p>{/if}
      {/if}
    </div>{/if}
  {#if error && !opened}<p role="alert">{error}</p>{/if}
</li>
