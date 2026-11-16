<script lang="ts">
  let { text, onOpen }: { text: string; onOpen: (name: string) => void } =
    $props();
  const lines = $derived(text.split("\n"));
  function parts(s: string) {
    return s.split(/(\[\[[^\]]+\]\])/g).map((t) => {
      const match = t.match(/^\[\[([^|\]]+)(?:\|([^\]]*))?\]\]$/);
      return match
        ? { text: match[2] || match[1].replaceAll("_", " "), name: match[1] }
        : { text: t, name: "" };
    });
  }
</script>

<div class="rich">
  {#if !text.trim()}<p class="muted">
      No description was recorded for this entry.
    </p>{:else}{#each lines as line}{#if line.trim()}<div
          class:rich-heading={line.startsWith("## ") || /^h[1-6]\./.test(line)}
          class:rich-bullet={/^\s*[-*]+\s/.test(line)}
        >
          {#each parts(line
              .replace(/^## |^h[1-6]\. /, "")
              .replace(/^\s*[-*]+\s/, "• ")) as part}{#if part.name}<button
                class="inline-link"
                onclick={() => onOpen(part.name)}>{part.text}</button
              >{:else}{part.text}{/if}{/each}
        </div>{:else}<div class="paragraph-gap"></div>{/if}{/each}{/if}
</div>
