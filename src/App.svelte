<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import RichText from "./RichText.svelte";
  import appIcon from "../src-tauri/icons/icon.png";
  import kofiIcon from "./assets/kofi.png";
  type Entry = {
    name: string;
    display: string;
    kind: string;
    coverage: string;
    children: number;
    custom?: boolean;
  };
  type Detail = Omit<Entry, "children"> & {
    description: string;
    children: Entry[];
    parents: Entry[];
    related: Entry[];
    crossReferences: Entry[];
    sourceUrl?: string;
    updated?: string;
    indexedAt?: string;
    edited: boolean;
  };
  type Snapshot = {
    version: number;
    label: string;
    created: string;
    protected: boolean;
    current: boolean;
    tags: number;
    categories: number;
    relationships: number;
    bytes: number;
  };
  type Job = {
    tags?: number;
    categories?: number;
    relationships?: number;
    status: string;
    message?: string;
    pages?: number;
    elapsed?: number;
    failures?: { at: string; message: string }[];
  };
  type Row = {
    entry: Entry;
    depth: number;
    key: string;
    cycle: boolean;
    trail: Entry[];
  };
  let snapshots: Snapshot[] = $state([]),
    version = $state(0),
    roots: Entry[] = $state([]),
    cache: Record<string, Entry[]> = $state({}),
    expanded: Set<string> = $state(new Set());
  let detail: Detail | null = $state(null),
    trail: Entry[] = $state([]),
    back: string[] = $state([]),
    forward: string[] = $state([]),
    recent: string[] = $state([]);
  let screen = $state("browse"),
    query = $state(""),
    kind = $state(""),
    hits: Entry[] = $state([]),
    searching = $state(false),
    filter = $state("");
  let notice = $state(""),
    busy = $state(false),
    theme = $state("dark"),
    progress = $state(false),
    job: Job = $state({ status: "idle" }),
    editing = $state(false),
    draft = $state("");
  let customOpen = $state(false),
    customName = $state(""),
    customKind = $state("tag"),
    customDescription = $state(""),
    linkName = $state(""),
    linkKind = $state("child");
  let left = $state(0),
    right = $state(0),
    diff: { label: string; total: number; sample: string[] }[] = $state([]),
    renameId = $state(0),
    renameLabel = $state("");
  let confirmAction: (() => Promise<void>) | null = $state(null),
    confirmMessage = $state(""),
    overwrite = $state(false),
    activateImport = $state(false);
  let scrollTop = $state(0),
    treeElement: HTMLDivElement | undefined = $state(),
    sidebar = $state(320),
    expanding = $state(false),
    pageLimit = $state(100);
  let requestId = 0,
    treeId = 0,
    searchId = 0;
  let timer: ReturnType<typeof setTimeout>;
  const active = $derived(["fetch", "build", "validate"].includes(job.status));
  const selectedSnapshot = $derived(
    snapshots.find((s) => (version ? s.version === version : s.current)),
  );
  const rows: Row[] = $derived.by(() => {
    const out: Row[] = [];
    const stack = roots
      .slice()
      .reverse()
      .map((entry) => ({
        entry,
        depth: 0,
        key: entry.name,
        cycle: false,
        trail: [] as Entry[],
      }));
    while (stack.length && out.length < 30000) {
      const row = stack.pop()!;
      out.push(row);
      if (expanded.has(row.key) && !row.cycle) {
        const children = cache[row.entry.name] || [];
        const ancestors = [...row.trail, row.entry];
        for (let i = children.length - 1; i >= 0; i--) {
          const child = children[i];
          stack.push({
            entry: child,
            depth: row.depth + 1,
            key: row.key + "\0" + child.name,
            cycle: ancestors.some((n) => n.name === child.name),
            trail: ancestors,
          });
        }
      }
    }
    return out;
  });
  const visible = $derived(
    rows.slice(
      Math.max(0, Math.floor(scrollTop / 38) - 8),
      Math.floor(scrollTop / 38) + 36,
    ),
  );
  const rowStart = $derived(Math.max(0, Math.floor(scrollTop / 38) - 8));
  async function api<T>(
    op: string,
    args: Record<string, unknown> = {},
  ): Promise<T> {
    return invoke("api", { op, args: { version, ...args } });
  }
  async function safely(fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      notice = String(e);
    }
  }
  function ask(message: string, action: () => Promise<void>) {
    confirmMessage = message;
    confirmAction = action;
  }
  async function refresh() {
    const meta = await api<{ snapshots: Snapshot[] }>("meta");
    snapshots = meta.snapshots;
    if (!snapshots.some((s) => s.version === left))
      left = snapshots[1]?.version || snapshots[0]?.version || 0;
    if (!snapshots.some((s) => s.version === right))
      right = snapshots[0]?.version || 0;
  }
  async function loadTree() {
    const id = ++treeId;
    requestId++;
    const next = await api<Entry[]>("children", { name: "" });
    if (id !== treeId) return;
    roots = next;
    cache = {};
    expanded = new Set();
    scrollTop = 0;
    if (treeElement) treeElement.scrollTop = 0;
  }
  async function switchVersion(v: number) {
    clearTimeout(timer);
    searchId++;
    searching = false;
    expanding = false;
    version = v;
    detail = null;
    trail = [];
    back = [];
    forward = [];
    hits = [];
    requestId++;
    await loadTree();
    if (query.trim()) await search();
  }
  async function select(name: string, history = false, parents: Entry[] = []) {
    const id = ++requestId;
    const node = await api<Detail>("node", { name });
    if (id !== requestId) return;
    if (detail && !history && detail.name !== name) {
      back = [...back, detail.name];
      forward = [];
    }
    detail = node;
    trail = parents;
    editing = false;
    draft = node.description;
    pageLimit = 100;
    recent = [name, ...recent.filter((n) => n !== name)].slice(0, 12);
    localStorage.setItem("recent", JSON.stringify(recent));
  }
  async function navigate(direction: "back" | "forward") {
    const list = direction === "back" ? back : forward;
    const name = list.at(-1);
    if (!name) return;
    if (direction === "back") {
      back = back.slice(0, -1);
      if (detail) forward = [...forward, detail.name];
    } else {
      forward = forward.slice(0, -1);
      if (detail) back = [...back, detail.name];
    }
    await select(name, true);
  }
  async function toggle(row: Row) {
    if (row.cycle) return;
    if (expanded.has(row.key)) {
      expanded = new Set([...expanded].filter((k) => k !== row.key));
      return;
    }
    const generation = treeId;
    if (!cache[row.entry.name]) {
      const children = await api<Entry[]>("children", { name: row.entry.name });
      if (generation !== treeId) return;
      cache = { ...cache, [row.entry.name]: children };
    }
    expanded = new Set([...expanded, row.key]);
  }
  async function expandAll() {
    if (expanding) {
      expanding = false;
      return;
    }
    expanding = true;
    const generation = treeId;
    const queue = roots.map((entry) => ({
      entry,
      key: entry.name,
      ancestors: [] as string[],
    }));
    let processed = 0;
    try {
      while (
        queue.length &&
        expanding &&
        generation === treeId &&
        processed < 1500
      ) {
        const row = queue.shift()!;
        if (row.ancestors.includes(row.entry.name)) continue;
        if (!cache[row.entry.name]) {
          const children = await api<Entry[]>("children", { name: row.entry.name });
          if (generation !== treeId || !expanding) break;
          cache = { ...cache, [row.entry.name]: children };
        }
        if (generation !== treeId) break;
        expanded = new Set([...expanded, row.key]);
        processed++;
        for (const entry of cache[row.entry.name])
          if (entry.children || entry.kind !== "tag")
            queue.push({
              entry,
              key: row.key + "\0" + entry.name,
              ancestors: [...row.ancestors, row.entry.name],
            });
      }
      if (queue.length && processed === 1500)
        notice =
          "Expanded 1,500 branches. Expand individual folders to continue; the full hierarchy remains available.";
    } finally {
      expanding = false;
    }
  }
  async function search() {
    const id = ++searchId;
    if (query.trim().length < 2) {
      hits = [];
      searching = false;
      return;
    }
    searching = true;
    try {
      const result = await api<Entry[]>("search", { query, kind });
      if (id === searchId) hits = result;
    } finally {
      if (id === searchId) searching = false;
    }
  }
  function scheduleSearch() {
    clearTimeout(timer);
    searchId++;
    timer = setTimeout(() => void safely(search), 180);
  }
  async function update(action: string) {
    job = await api<Job>("update", { action });
    progress = true;
  }
  async function snapshot(action: string, s: Snapshot) {
    await api("snapshot", {
      version: s.version,
      action,
      label: renameLabel,
      confirm: action === "delete",
    });
    renameId = 0;
    await refresh();
    if (action === "restore" || action === "delete") {
      await switchVersion(0);
    }
    notice = "Snapshot updated locally.";
  }
  async function exportArchive(mode: string, v = version) {
    const path = await save({
      defaultPath: `danbooru-tag-indexer-${mode}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
    });
    if (!path) return;
    busy = true;
    try {
      await api("export", { path, mode, version: v });
      notice = "Archive saved: " + path;
    } finally {
      busy = false;
    }
  }
  async function importArchive() {
    const path = await open({
      multiple: false,
      filters: [{ name: "Danbooru Tag Indexer archive", extensions: ["zip"] }],
    });
    if (!path || Array.isArray(path)) return;
    busy = true;
    try {
      const result = await api<{ imported: number }>("import", {
        path,
        overwrite,
        activate: activateImport,
      });
      await refresh();
      await switchVersion(0);
      notice = `Imported ${result.imported} snapshots and custom edits.`;
    } finally {
      busy = false;
    }
  }
  async function saveDescription() {
    if (!detail) return;
    await api("edit", { name: detail.name, description: draft });
    await select(detail.name, true);
  }
  async function create() {
    const name = await api<string>("create", {
      name: customName,
      kind: customKind,
      description: customDescription,
      parent: detail?.name || "tag_groups",
    });
    customOpen = false;
    customName = "";
    customDescription = "";
    await loadTree();
    await select(name);
  }
  async function relationship(remove = false) {
    if (!detail) return;
    await api("edge", {
      parent: detail.name,
      child: linkName,
      kind: linkKind,
      remove,
    });
    await loadTree();
    await select(detail.name, true);
    linkName = "";
  }
  function resize(e: PointerEvent) {
    const start = e.clientX,
      old = sidebar;
    const move = (ev: PointerEvent) =>
      (sidebar = Math.max(230, Math.min(600, old + ev.clientX - start)));
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
  }
  onMount(() => {
    theme = localStorage.getItem("theme") === "light" ? "light" : "dark";
    try {
      const stored = JSON.parse(localStorage.getItem("recent") || "[]");
      recent = Array.isArray(stored) ? stored.filter((x): x is string => typeof x === "string").slice(0, 12) : [];
    } catch {
      recent = [];
    }
    void safely(async () => {
      await refresh();
      await loadTree();
      job = await api<Job>("job");
    }).finally(async () => {
      // Show even when loading fails, so the error remains visible and actionable.
      await tick();
      // Hidden webviews may suspend animation frames; wait for Svelte's DOM
      // update instead so first launch cannot become stuck behind a hidden window.
      await invoke("app_ready");
    });
    const poll = setInterval(() => {
      if (active || progress)
        void safely(async () => {
          const prior = job.status;
          job = await api<Job>("job");
          if (prior !== "done" && job.status === "done") {
            await refresh();
            await switchVersion(0);
            notice = "New index validated and activated.";
          }
        });
    }, 1000);
    return () => {
      clearInterval(poll);
      clearTimeout(timer);
    };
  });
</script>

<div class="app" data-theme={theme}>
  <header>
    <div class="brand">
      <img class="mark" src={appIcon} alt="" />
      <div>
        <strong>Danbooru Tag Indexer</strong><small>THE OFFLINE ENCYCLOPEDIA</small>
      </div>
    </div>
    <nav>
      <button
        class:chosen={screen === "browse"}
        onclick={() => (screen = "browse")}>Encyclopedia</button
      ><button
        class:chosen={screen === "history"}
        onclick={() => {
          screen = "history";
          void safely(refresh);
        }}>Index history</button
      >
    </nav>
    <div class="header-actions">
      <button
        class="support"
        title="Support Me on Ko-fi (opens in your browser)"
        onclick={() => void safely(() => openUrl("https://ko-fi.com/forbidden_fantasy"))}
        ><img src={kofiIcon} alt="" />Support Me</button
      ><button
        title="Toggle theme"
        onclick={() => {
          theme = theme === "dark" ? "light" : "dark";
          localStorage.setItem("theme", theme);
        }}>{theme === "dark" ? "☀" : "☾"}</button
      ><button
        class="primary"
        disabled={active}
        onclick={() => void safely(() => update("start"))}
        >↻ Update Index</button
      >
    </div>
  </header>
  {#if notice}<div class="notice" role="status">
      <span>{notice}</span><button
        aria-label="Dismiss message"
        onclick={() => (notice = "")}>×</button
      >
    </div>{/if}
  {#if screen === "browse"}
    <div class="toolbar">
      <button
        aria-label="Back"
        disabled={!back.length}
        onclick={() => void safely(() => navigate("back"))}>←</button
      ><button
        aria-label="Forward"
        disabled={!forward.length}
        onclick={() => void safely(() => navigate("forward"))}>→</button
      >
      <div class="search">
        <span>⌕</span><input
          aria-label="Search encyclopedia"
          placeholder="Search tags, descriptions, and relationships…"
          bind:value={query}
          oninput={scheduleSearch}
        />{#if query}<button
            aria-label="Clear search"
            onclick={() => {
              query = "";
              hits = [];
              searchId++;
              searching = false;
              clearTimeout(timer);
            }}>×</button
          >{/if}<select
          aria-label="Search type"
          bind:value={kind}
          onchange={scheduleSearch}
          ><option value="">All entries</option><option value="tag">Tags</option
          ><option value="group">Categories</option></select
        >
      </div>
      <select
        aria-label="Index version"
        value={version}
        onchange={(e) =>
          void safely(() => switchVersion(Number(e.currentTarget.value)))}
        ><option value={0}>Current index</option>{#each snapshots as s}<option
            value={s.version}>{s.label}{s.protected ? " · kept" : ""}</option
          >{/each}</select
      ><button onclick={() => (customOpen = true)}>＋ Custom entry</button>
    </div>
    <div class="workspace" style={`--sidebar:${sidebar}px`}>
      <aside>
        <div class="aside-title">
          <span>CATEGORY EXPLORER</span><small
            >{selectedSnapshot?.categories.toLocaleString() || "…"}</small
          >
        </div>
        <input
          class="filter"
          aria-label="Filter categories"
          placeholder="Find a category…"
          bind:value={filter}
          oninput={() => {
            query = filter;
            kind = filter ? "group" : "";
            scheduleSearch();
          }}
        />
        <div class="tree-actions">
          <button onclick={() => void safely(expandAll)}
            >{expanding ? "Stop expanding" : "Expand all"}</button
          ><button
            onclick={() => {
              expanding = false;
              expanded = new Set();
              scrollTop = 0;
              if (treeElement) treeElement.scrollTop = 0;
            }}>Collapse all</button
          >
        </div>
        <div
          class="tree"
          bind:this={treeElement}
          onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
        >
          <div style={`height:${rows.length * 38}px;position:relative`}>
            {#each visible as row, i (row.key)}<div
                class:selected={detail?.name === row.entry.name}
                class="tree-row"
                style={`top:${(rowStart + i) * 38}px;padding-left:${12 + row.depth * 18}px`}
              >
                <button
                  class="chevron"
                  aria-label={`Expand ${row.entry.display}`}
                  disabled={row.cycle}
                  onclick={() => void safely(() => toggle(row))}
                  >{row.cycle
                    ? "↩"
                    : expanded.has(row.key)
                      ? "⌄"
                      : row.entry.children || row.entry.kind !== "tag"
                        ? "›"
                        : "·"}</button
                ><button
                  class="tree-label"
                  title={row.entry.display}
                  onclick={() =>
                    void safely(() => select(row.entry.name, false, row.trail))}
                  ><span class="folder"
                    >{row.entry.kind === "tag" ? "#" : "◇"}</span
                  >{row.entry.display}</button
                >
              </div>{/each}
          </div>
        </div>
        <footer>
          <span>Stored on this device</span><button
            onclick={() => (progress = true)}>Update status</button
          >
        </footer>
      </aside>
      <button
        class="resizer"
        aria-label="Resize category panel"
        tabindex="0"
        onpointerdown={resize}
        onkeydown={(e) => {
          if (e.key === "ArrowLeft") sidebar = Math.max(230, sidebar - 20);
          if (e.key === "ArrowRight") sidebar = Math.min(600, sidebar + 20);
        }}
      ></button>
      <main>
        {#if query.trim().length >= 2}<section class="results">
            <div class="eyebrow">LOCAL SEARCH</div>
            <h2>Results for “{query}”</h2>
            <p class="muted">
              {searching
                ? "Searching…"
                : `${hits.length} matching entries${hits.length >= 80 ? " · refine your query for more precise results" : ""}`}
            </p>
            <div class="result-grid">
              {#each hits as hit}<button
                  onclick={() =>
                    void safely(async () => {
                      await select(hit.name);
                      query = "";
                      filter = "";
                      hits = [];
                    })}
                  ><span>{hit.display}</span><small>{hit.kind}</small></button
                >{/each}
            </div>
          </section>
        {:else if detail}<div class="breadcrumbs">
            <button
              onclick={() => {
                detail = null;
                trail = [];
              }}>Encyclopedia</button
            >{#each trail as parent}<span>›</span><button
                onclick={() => void safely(() => select(parent.name))}
                >{parent.display}</button
              >{/each}<span>›</span><span>{detail.display}</span>
          </div>
          <article>
            <div class="eyebrow">
              {detail.custom ? "CUSTOM ENTRY" : detail.kind.toUpperCase()}
              {detail.edited ? "· LOCALLY EDITED" : ""}
            </div>
            <div class="title-row">
              <h1>{detail.display}</h1>
              <button
                onclick={() => {
                  editing = !editing;
                  draft = detail?.description || "";
                }}>{editing ? "Cancel edit" : "Edit description"}</button
              >
            </div>
            <div class="entry-name">{detail.name}</div>
            {#if detail.coverage === "unindexed"}<div class="callout">
                This name is in the hierarchy, but its wiki description was
                unavailable in the collected data. Nothing will be downloaded
                when you open it.
              </div>{/if}
            {#if editing}<textarea
                aria-label="Description"
                class="editor"
                bind:value={draft}></textarea>
              <div class="actions">
                <button
                  class="primary"
                  onclick={() => void safely(saveDescription)}
                  >Save local edit</button
                >{#if detail.edited}<button
                    onclick={() =>
                      ask(
                        "Remove your local description and use the original?",
                        async () => {
                          await api("revert", { name: detail!.name });
                          await select(detail!.name, true);
                        },
                      )}>Revert to original</button
                  >{/if}
              </div>{:else}<RichText
                text={detail.description}
                onOpen={(name) => void safely(() => select(name))}
              />{/if}
            {#each [{ label: "Parent categories", entries: detail.parents }, { label: "Child categories & tags", entries: detail.children }, { label: "Related tags", entries: detail.related }, { label: "Cross-references", entries: detail.crossReferences }] as group}{#if group.entries.length}<section
                  class="links-section"
                >
                  <h3>{group.label}<small>{group.entries.length}</small></h3>
                  <div class="chips">
                    {#each group.entries.slice(0, pageLimit) as entry}<button
                        onclick={() =>
                          void safely(() =>
                            select(
                              entry.name,
                              false,
                              group.label === "Child categories & tags"
                                ? [...trail, detail as unknown as Entry]
                                : [],
                            ),
                          )}
                        >{entry.kind === "tag" ? "#" : "◇"}
                        {entry.display}</button
                      >{/each}
                  </div>
                  {#if group.entries.length > pageLimit}<button
                      onclick={() => (pageLimit += 200)}>Show more</button
                    >{/if}
                </section>{/if}{/each}
            <details class="custom-tools">
              <summary>Custom relationships</summary>
              <p class="muted">
                Add or remove your own links. Original relationships remain
                unchanged.
              </p>
              <div class="actions">
                <input
                  aria-label="Relationship target"
                  placeholder="Exact local tag or category name"
                  bind:value={linkName}
                /><select aria-label="Relationship type" bind:value={linkKind}
                  ><option value="child">Child</option><option value="related"
                    >Related</option
                  ><option value="xref">Cross-reference</option></select
                ><button
                  disabled={!linkName.trim()}
                  onclick={() => void safely(() => relationship())}>Add</button
                ><button
                  disabled={!linkName.trim()}
                  onclick={() => void safely(() => relationship(true))}
                  >Remove custom link</button
                >
              </div>
            </details>
            <div class="source-line">
              {#if detail.indexedAt}<span
                  >Indexed {new Date(detail.indexedAt).toLocaleString()}</span
                >{/if}
              <span
                >{detail.updated
                  ? "Source updated " +
                    new Date(detail.updated).toLocaleDateString()
                  : "No source modification date"}</span
              >{#if detail.sourceUrl}<button
                  onclick={() => void safely(() => openUrl(detail!.sourceUrl!))}
                  >Open original on Danbooru ↗</button
                >{/if}{#if detail.custom}<button
                  class="danger"
                  onclick={() =>
                    ask(
                      "Delete this custom entry and its custom relationships?",
                      async () => {
                        await api("deleteCustom", { name: detail!.name });
                        detail = null;
                        await loadTree();
                      },
                    )}>Delete custom entry</button
                >{/if}
            </div>
          </article>
        {:else}<section class="welcome">
            <div class="eyebrow">YOUR LOCAL REFERENCE LIBRARY</div>
            <h1>A world of tags.<br /><em>Always within reach.</em></h1>
            <p>
              Explore the connections between visual ideas. Every category,
              description, and relationship is available on your device.
            </p>
            <div class="stats">
              <div>
                <strong>{selectedSnapshot?.tags.toLocaleString() || "…"}</strong
                ><span>indexed tags</span>
              </div>
              <div>
                <strong
                  >{selectedSnapshot?.categories.toLocaleString() ||
                    "…"}</strong
                ><span>categories</span>
              </div>
              <div>
                <strong
                  >{selectedSnapshot?.relationships.toLocaleString() ||
                    "…"}</strong
                ><span>relationships</span>
              </div>
            </div>
            <h3>Start exploring</h3>
            <div class="root-cards">
              {#each roots.filter((r) => !r.name.includes("#intro")) as root}<button
                  onclick={() => void safely(() => select(root.name))}
                  ><span>◇</span><strong>{root.display}</strong><small
                    >{root.children} branches →</small
                  ></button
                >{/each}
            </div>
            {#if recent.length}<h3>Recently viewed</h3>
              <div class="chips">
                {#each recent as name}<button
                    onclick={() => void safely(() => select(name))}
                    >{name.replaceAll("_", " ")}</button
                  >{/each}
              </div>{/if}
            <div class="callout">
              Browsing stays offline. Only <strong>Update Index</strong> starts a
              download. Missing descriptions are left blank.
            </div>
          </section>{/if}
      </main>
    </div>
  {:else}
    <main class="history">
      <div class="eyebrow">LOCAL VERSION HISTORY</div>
      <h1>Your index, over time.</h1>
      <p class="muted">
        Keep the current index and two previous unprotected versions. Protected
        snapshots stay until you choose to delete them.
      </p>
      <div class="table-wrap">
        <table>
          <thead
            ><tr
              ><th>Version</th><th>Created</th><th>Tags / categories</th><th
                >Size</th
              ><th>Status & actions</th></tr
            ></thead
          ><tbody
            >{#each snapshots as s}<tr
                ><td
                  ><strong>{s.label}</strong><small>Version {s.version}</small
                  >{#if renameId === s.version}<input
                      aria-label="Snapshot name"
                      bind:value={renameLabel}
                    /><button
                      onclick={() => void safely(() => snapshot("rename", s))}
                      >Save</button
                    >{/if}</td
                ><td>{new Date(s.created).toLocaleString()}</td><td
                  >{s.tags.toLocaleString()} / {s.categories.toLocaleString()}<small
                    >{s.relationships.toLocaleString()} relationships</small
                  ></td
                ><td>{(s.bytes / 1e6).toFixed(1)} MB</td><td
                  ><span class="badge"
                    >{s.current ? "Current" : "Previous"}{s.protected
                      ? " · Protected"
                      : ""}</span
                  >
                  <div class="actions">
                    <button
                      onclick={() =>
                        void safely(async () => {
                          await switchVersion(s.version);
                          screen = "browse";
                        })}>Open</button
                    ><button
                      onclick={() =>
                        void safely(() =>
                          snapshot(s.protected ? "unprotect" : "protect", s),
                        )}
                      >{s.protected
                        ? "Remove protection"
                        : "Keep permanently"}</button
                    ><button
                      onclick={() => {
                        renameId = s.version;
                        renameLabel = s.label;
                      }}>Rename</button
                    ><button
                      disabled={busy}
                      onclick={() =>
                        void safely(() => exportArchive("snapshot", s.version))}
                      >Export</button
                    >{#if !s.current}<button
                        onclick={() =>
                          ask(
                            "Restore this snapshot as a new current version? No network is used.",
                            () => snapshot("restore", s),
                          )}>Restore</button
                      ><button
                        class="danger"
                        onclick={() =>
                          ask(
                            `Permanently delete ${s.label}${s.protected ? " (protected)" : ""}?`,
                            () => snapshot("delete", s),
                          )}>Delete</button
                      >{/if}
                  </div></td
                ></tr
              >{/each}</tbody
          >
        </table>
      </div>
      <section class="panel">
        <h2>Compare snapshots</h2>
        <div class="actions">
          <select aria-label="Older snapshot" bind:value={left}
            >{#each snapshots as s}<option value={s.version}>{s.label}</option
              >{/each}</select
          ><span>→</span><select aria-label="Newer snapshot" bind:value={right}
            >{#each snapshots as s}<option value={s.version}>{s.label}</option
              >{/each}</select
          ><button
            class="primary"
            onclick={() =>
              void safely(async () => {
                busy = true;
                try {
                  diff = await api("compare", { left, right });
                } finally {
                  busy = false;
                }
              })}
            disabled={busy}>Compare versions</button
          >
        </div>
        <div class="diff-grid">
          {#each diff as item}<details>
              <summary
                >{item.label}
                <strong>{item.total.toLocaleString()}</strong></summary
              >
              <p>{item.sample.join(", ") || "No changes"}</p>
              {#if item.total > 150}<small>Showing the first 150 changes.</small
                >{/if}
            </details>{/each}
        </div>
      </section>
      <section class="panel">
        <h2>Portable backups</h2>
        <p class="muted">
          ZIP archives contain independent snapshots and your custom edits.
          Imports are validated before being applied.
        </p>
        <div class="actions">
          <button
            disabled={busy}
            onclick={() => void safely(() => exportArchive("history"))}
            >Export full history</button
          ><button
            disabled={busy}
            onclick={() => void safely(() => exportArchive("overlay"))}
            >Export custom edits</button
          >
        </div>
        <div class="import-options">
          <label
            ><input type="checkbox" bind:checked={activateImport} /> Make the imported
            current snapshot active</label
          ><label
            ><input type="checkbox" bind:checked={overwrite} /> Replace conflicting
            custom edits with archive contents</label
          >
        </div>
        <button disabled={busy} onclick={() => void safely(importArchive)}
          >{busy ? "Working…" : "Import ZIP archive"}</button
        >
      </section>
    </main>
  {/if}
  {#if progress}<div class="modal-backdrop">
      <div
        class="modal"
        role="dialog"
        aria-modal="true"
        aria-label="Update progress"
        tabindex="-1"
      >
        <div class="title-row">
          <h2>Update index</h2>
          <button aria-label="Close progress" onclick={() => (progress = false)}
            >×</button
          >
        </div>
        <span class="badge">{job.status}</span>
        <p>{job.message || "No downloads run until you click Update Index."}</p>
        <div class="stats">
          <div>
            <strong>{(job.pages || 0).toLocaleString()}</strong><span
              >wiki pages processed</span
            >
          </div>
          <div>
            <strong>{Math.floor((job.elapsed || 0) / 60)}m</strong><span
              >elapsed time</span
            >
          </div>
        </div>
        <p class="muted">
          {(job.tags || 0).toLocaleString()} tags · {(
            job.categories || 0
          ).toLocaleString()} categories · {(
            job.relationships || 0
          ).toLocaleString()} relationships discovered
        </p>
        <p class="muted">
          The active index stays untouched until the new database passes
          validation. The final number of pages is not known in advance.
        </p>
        <div class="actions">
          {#if active}<button onclick={() => void safely(() => update("pause"))}
              >Pause update</button
            ><button onclick={() => void safely(() => update("cancel"))}
              >Cancel update</button
            >{:else if ["paused", "interrupted", "error"].includes(job.status)}<button
              class="primary"
              onclick={() => void safely(() => update("resume"))}
              >Resume update</button
            >{#if job.status === "error"}<button
                onclick={() => void safely(() => update("retry"))}
                >Retry failed request</button
              >{/if}<button onclick={() => void safely(() => update("cancel"))}
              >Discard update</button
            >{/if}
        </div>
        {#if job.failures?.length}<details>
            <summary>View errors ({job.failures.length})</summary
            >{#each job.failures as failure}<p>
                {failure.at}: {failure.message}
              </p>{/each}
          </details>{/if}
      </div>
    </div>{/if}
  {#if customOpen}<div class="modal-backdrop">
      <form
        class="modal"
        onsubmit={(e) => {
          e.preventDefault();
          void safely(create);
        }}
      >
        <div class="title-row">
          <h2>Create custom entry</h2>
          <button type="button" onclick={() => (customOpen = false)}>×</button>
        </div>
        <p class="muted">
          Added under {detail?.display || "the encyclopedia root"}. Custom
          information persists across updates.
        </p>
        <label>Name<input required bind:value={customName} /></label><label
          >Type<select bind:value={customKind}
            ><option value="tag">Tag</option><option value="group"
              >Category</option
            ></select
          ></label
        ><label
          >Description<textarea bind:value={customDescription}
          ></textarea></label
        ><button class="primary">Create entry</button>
      </form>
    </div>{/if}
  {#if confirmAction}<div class="modal-backdrop">
      <div
        class="modal"
        role="alertdialog"
        aria-modal="true"
        aria-label="Confirm action"
        tabindex="-1"
      >
        <h2>Confirm action</h2>
        <p>{confirmMessage}</p>
        <div class="actions">
          <button onclick={() => (confirmAction = null)}>Cancel</button><button
            class="primary"
            onclick={() => {
              const fn = confirmAction;
              confirmAction = null;
              if (fn) void safely(fn);
            }}>Confirm</button
          >
        </div>
      </div>
    </div>{/if}
</div>
