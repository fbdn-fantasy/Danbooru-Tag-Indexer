<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import WikiTree from "./WikiTree.svelte";
  import {
    shortDescription,
    type WikiEntry,
    type WikiPage,
    type Favorite,
    type TreeActions,
  } from "./navigation";
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
  let snapshots: Snapshot[] = $state([]),
    version = $state(0),
    roots: WikiEntry[] = $state([]);
  let treeCommand = $state({ serial: 0, open: false }),
    treeGeneration = $state(0);
  let outlineCache = new Map<string, Promise<WikiPage>>();
  let detailPage: WikiPage = $state({ description: "", entries: [] });
  let favorites: Favorite[] = $state([]),
    favoriteQuery = $state("");
  let favoriteBusy = new Set<string>();
  const stars = $derived(new Set(favorites.map((f) => f.name)));
  const favoriteRows = $derived(
    favorites.filter((f) =>
      (
        f.name +
        " " +
        f.display +
        " " +
        f.description +
        " " +
        f.paths
          .flat()
          .map((p) => p.display)
          .join(" ")
      )
        .toLowerCase()
        .includes(favoriteQuery.toLowerCase()),
    ),
  );
  async function loadOutline(name: string): Promise<WikiPage> {
    const key = `${version}:${name}`;
    if (!outlineCache.has(key))
      outlineCache.set(
        key,
        api<WikiPage>("outline", { name }).catch((e) => {
          outlineCache.delete(key);
          throw e;
        }),
      );
    return outlineCache.get(key)!;
  }
  async function copyTag(name: string) {
    await navigator.clipboard.writeText(name.replaceAll("_", " "));
    notice = "Copied " + name.replaceAll("_", " ");
  }
  async function starTag(name: string, path: WikiEntry[] = []) {
    if (favoriteBusy.has(name)) return;
    favoriteBusy.add(name);
    try {
      favorites = await api<Favorite[]>("favorite", {
        name,
        path,
        remove: stars.has(name),
      });
    } finally {
      favoriteBusy.delete(name);
    }
  }
  const treeActions: TreeActions = {
    load: loadOutline,
    copy: copyTag,
    star: starTag,
    open: (name) => void safely(() => select(name)),
  };
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
  let sidebar = $state(320);
  let requestId = 0,
    treeId = 0,
    searchId = 0;
  let timer: ReturnType<typeof setTimeout>;
  const active = $derived(["fetch", "build", "validate"].includes(job.status));
  const selectedSnapshot = $derived(
    snapshots.find((s) => (version ? s.version === version : s.current)),
  );
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
    outlineCache = new Map();
    const next = await loadOutline("tag_groups");
    if (id !== treeId) return;
    roots = next.entries;
    treeGeneration++;
  }
  async function switchVersion(v: number) {
    clearTimeout(timer);
    searchId++;
    searching = false;
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
    const [node, page] = await Promise.all([
      api<Detail>("node", { name }),
      loadOutline(name),
    ]);
    if (id !== requestId) return;
    if (detail && !history && detail.name !== name) {
      back = [...back, detail.name];
      forward = [];
    }
    detail = node;
    detailPage = page;
    trail = parents;
    editing = false;
    draft = node.description;
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
    outlineCache.clear();
    treeGeneration++;
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
      recent = Array.isArray(stored)
        ? stored.filter((x): x is string => typeof x === "string").slice(0, 12)
        : [];
    } catch {
      recent = [];
    }
    void safely(async () => {
      await refresh();
      favorites = await api<Favorite[]>("favorites");
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
        <strong>Danbooru Tag Indexer</strong><small
          >THE OFFLINE ENCYCLOPEDIA</small
        >
      </div>
    </div>
    <nav>
      <button
        class:chosen={screen === "browse"}
        onclick={() => (screen = "browse")}>Encyclopedia</button
      ><button
        class:chosen={screen === "favorites"}
        onclick={() => {
          screen = "favorites";
          void safely(async () => {
            favorites = await api<Favorite[]>("favorites");
          });
        }}>Favorites <small>{favorites.length}</small></button
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
        onclick={() =>
          void safely(() => openUrl("https://ko-fi.com/forbidden_fantasy"))}
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
          <button
            onclick={() =>
              (treeCommand = { serial: treeCommand.serial + 1, open: true })}
            >Expand loaded</button
          >
          <button
            onclick={() =>
              (treeCommand = { serial: treeCommand.serial + 1, open: false })}
            >Collapse all</button
          >
        </div>
        <div class="tree">
          {#key treeGeneration}<WikiTree
              entries={roots}
              actions={treeActions}
              {stars}
              command={treeCommand}
              ancestors={["tag_groups"]}
            />{/key}
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
            <div class="search-outlines">
              {#key `${treeGeneration}:${query}:${kind}`}<WikiTree
                  entries={hits.map((h) => ({
                    ...h,
                    entries: [],
                    description: "",
                    note: "",
                  }))}
                  actions={treeActions}
                  {stars}
                />{/key}
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
              {#if detail.kind === "tag"}<button
                  onclick={() => void safely(() => copyTag(detail!.name))}
                  >Copy</button
                ><button
                  aria-label={`${stars.has(detail.name) ? "Unstar" : "Star"} ${detail.display}`}
                  aria-pressed={stars.has(detail.name)}
                  onclick={() => void safely(() => starTag(detail!.name))}
                  >{stars.has(detail.name) ? "★" : "☆"}</button
                >{/if}
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
              </div>{:else}
              {#if detailPage.description}<RichText
                  text={detailPage.description}
                  onOpen={(name) => void safely(() => select(name))}
                />{/if}
              {#key `${treeGeneration}:${detail.name}:${detail.description}`}<WikiTree
                  entries={detailPage.entries}
                  actions={treeActions}
                  {stars}
                  ancestors={[detail.name]}
                  openSections={true}
                />{/key}
            {/if}
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
                    >{root.entries.length} branches →</small
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
  {:else if screen === "favorites"}
    <main class="favorites">
      <div class="eyebrow">YOUR SAVED TAGS</div>
      <h1>Favorites</h1>
      <input
        aria-label="Search favorites"
        placeholder="Search saved tags, descriptions, or folders…"
        bind:value={favoriteQuery}
      />
      {#if !favoriteRows.length}<p class="muted">
          {favoriteQuery
            ? "No saved tags match your search."
            : "Star tags while browsing or searching to keep them here."}
        </p>{/if}
      {#each favoriteRows as favorite (favorite.name)}
        <article class="favorite-card" data-name={favorite.name}>
          <div class="favorite-actions">
            <button
              aria-label={`Copy ${favorite.display}`}
              onclick={() => void safely(() => copyTag(favorite.name))}
              >Copy</button
            ><button
              class="starred"
              aria-label={`Unstar ${favorite.display}`}
              onclick={() => void safely(() => starTag(favorite.name))}
              >★</button
            >
          </div>
          <details>
            <summary
              ><strong>{favorite.display}</strong><span class="favorite-summary"
                >{shortDescription(favorite.description) ||
                  "No description was recorded."}</span
              ></summary
            >
            <div class="favorite-details">
              {#each favorite.paths as path}<p class="favorite-path">
                  {path.map((p) => p.display).join(" › ")}
                </p>{/each}
              <RichText
                text={favorite.description}
                onOpen={(name) => {
                  screen = "browse";
                  query = "";
                  void safely(() => select(name));
                }}
              />
            </div>
          </details>
        </article>
      {/each}
    </main>
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
