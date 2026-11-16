<script lang="ts">
  import { wikiText } from "./navigation";
  let { text, onOpen }: { text: string; onOpen: (name: string) => void } =
    $props();
  const lines = $derived(wikiText(text).split("\n"));
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
          class:rich-heading={/^#{1,6} /.test(line) || /^h[1-6]\./.test(line)}
          class:rich-bullet={/^\s*[-*]+\s/.test(line)}
          style:padding-left={/^\s*-/.test(line)
            ? `${(line.match(/^ */)?.[0].length || 0) * 8}px`
            : undefined}
        >
          {#each parts(line
              .replace(/^#{1,6} |^h[1-6]\. /, "")
              .replace(/^\s*[-*]+\s/, "• ")) as part}{#if part.name}<button
                class="inline-link"
                onclick={() => onOpen(part.name)}>{part.text}</button
              >{:else}{part.text}{/if}{/each}
        </div>{:else}<div class="paragraph-gap"></div>{/if}{/each}{/if}
</div>
