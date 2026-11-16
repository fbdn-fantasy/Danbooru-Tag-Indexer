<script lang="ts">
  import WikiBranch from "./WikiBranch.svelte";
  import type { WikiEntry, TreeActions } from "./navigation";
  let {
    entries,
    actions,
    stars,
    trail = [],
    ancestors = [],
    openSections = false,
    command = { serial: 0, open: false },
  }: {
    entries: WikiEntry[];
    actions: TreeActions;
    stars: Set<string>;
    trail?: WikiEntry[];
    ancestors?: string[];
    openSections?: boolean;
    command?: { serial: number; open: boolean };
  } = $props();
</script>

<ul class="wiki-tree">
  {#each entries as entry, i (`${entry.name}:${i}`)}
    <WikiBranch
      {entry}
      {actions}
      {stars}
      {trail}
      {ancestors}
      {openSections}
      {command}
    />
  {/each}
</ul>
