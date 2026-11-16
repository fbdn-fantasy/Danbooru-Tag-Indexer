export type WikiEntry = {
  name: string;
  display: string;
  kind: string;
  entries: WikiEntry[];
  description: string;
  note: string;
};
export type WikiPage = { description: string; entries: WikiEntry[] };
export type Favorite = {
  name: string;
  display: string;
  description: string;
  paths: { name: string; display: string }[][];
};
export type TreeActions = {
  load: (name: string) => Promise<WikiPage>;
  copy: (name: string) => Promise<void>;
  star: (name: string, path: WikiEntry[]) => Promise<void>;
  open: (name: string) => void;
};

// Keep source occurrences in the full wiki, adding only unique preview content.
// If a duplicate preview parent has unique descendants, merge into that parent.
export function mergePreview(
  full: WikiEntry[],
  preview: WikiEntry[],
): WikiEntry[] {
  const clone = (e: WikiEntry): WikiEntry => ({
    ...e,
    entries: e.entries.map(clone),
  });
  const result = full.map(clone);
  const find = (entries: WikiEntry[], name: string): WikiEntry | undefined => {
    for (const e of entries) {
      if (e.name === name) return e;
      const nested = find(e.entries, name);
      if (nested) return nested;
    }
  };
  const add = (entries: WikiEntry[], target: WikiEntry[]) => {
    for (const e of entries) {
      const existing = find(result, e.name);
      if (existing) add(e.entries, existing.entries);
      else {
        const next = { ...e, entries: [] as WikiEntry[] };
        target.push(next);
        add(e.entries, next.entries);
      }
    }
  };
  add(preview, result);
  return result;
}
export function loadsWiki(entry: WikiEntry) {
  return (
    entry.kind === "tag" || (entry.kind !== "section" && !entry.entries.length)
  );
}
export function shortDescription(text: string) {
  return text
    .split(/\n#{2,6}\s/)[0]
    .replace(/\[\[([^|\]]+)(?:\|([^\]]+))?\]\]/g, (_, a, b) =>
      (b || a).replaceAll("_", " "),
    )
    .replace(/\s+/g, " ")
    .trim();
}

export function wikiText(text: string) {
  return text.replace(
    /"([^"]+)":\[?(?:https?:\/\/[^/\s]+)?\/wiki_pages\/([^#?\]\s]+)(?:#[^\]\s]+)?\]?/g,
    (original, label, name) => {
      try {
        return `[[${decodeURIComponent(name)}|${label}]]`;
      } catch {
        return original;
      }
    },
  );
}
