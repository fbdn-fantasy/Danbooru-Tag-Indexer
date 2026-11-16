# Building version 5.0.0

The application retains its Rust/Tauri 2 backend and Svelte 5/TypeScript frontend.

## Source and data

Release source ZIPs include `src-tauri/resources/codex-v1.sqlite.gz`. Run `npm ci`
and `npm run prepare:data` to decompress the bundled offline catalog. For a Git
checkout, copy that compressed resource from a release source ZIP first. Generated
SQLite files, build output, dependencies, and personal data are not committed.

## Checks

```sh
npm run check
node --test tests/navigation.test.mjs
npm run build
cd src-tauri
cargo test --locked
```

The tests use the bundled database and public wiki fixtures captured for this
release. `scripts/verify-wiki.py` deliberately refreshes those fixtures from
Danbooru; it is a development tool and never runs while browsing the app.
The optional full-dump rebuild test requires a separate complete raw wiki dump.

`tests/native_navigation.py` drives the real Linux Tauri window using
WebKitWebDriver. Set `APP_BINARY`, `TEST_OUTPUT`, `XDG_DATA_HOME`, and
`XDG_CONFIG_HOME` to isolated test paths, start WebKitWebDriver on port 4445,
and run it under an X11 display. Never point these tests at personal app data.

## Installable packages

Windows builds require the Rust MSVC target and Tauri's Windows prerequisites:

```sh
npm run tauri -- build --config src-tauri/tauri.windows.conf.json
```

The NSIS installer bundles the database, app assets, and offline WebView2 runtime
installer. It keeps the existing `io.tagcodex.desktop` application identity.

Linux builds require GTK 3, WebKitGTK 4.1, and their development packages:

```sh
npm run tauri -- build --bundles deb,rpm
```

Package dependencies arrange GTK/WebKit and system runtime installation. Neither
platform requires a source archive or adjacent support files after installation.

For a Linux-hosted Windows cross-build, install `cargo-xwin`, the
`x86_64-pc-windows-msvc` Rust target, Clang/LLD, LLVM resource tools, and NSIS:

```sh
npm run tauri -- build --runner cargo-xwin --target x86_64-pc-windows-msvc --config src-tauri/tauri.windows.conf.json
```

## Navigation and saved data

`navigation.rs` reconstructs each wiki from its stored description and section
metadata. Category links with explicit children retain those children; other
categories load their linked wiki on expansion. Tag labels open the main article
and reveal their inline list; arrows expand or collapse without changing the
main article. Search-result tag selection exits the results view. Real tags load their complete
wiki and merge unique preview descendants without synthetic headings. Page
requests are cached per index version, with per-branch ancestor guards for cycles.

Older catalogs remain readable without rewriting their IDs, graph, or snapshots.
On the first V5 launch, the exact original V4 bundled catalog is upgraded to a
new snapshot; its old snapshot remains available. Downloaded or imported indexes
are not replaced. Restoring an older snapshot later is respected.
The importer is scoped to `tag_groups` and the category and tag wikis reached
recursively through their listed links. It preserves heading levels and full
lists on ordinary tag pages. Unrelated articles from the raw download are not
included as full wiki pages. Favorites are snapshot records in the existing local registry database,
including descriptions and browsing paths. They survive index replacement and
are included in ZIP backups; backups from before favorites remain importable.
