# Danbooru Tag Indexer

An open-source desktop application for browsing, searching, and managing an offline index of Danbooru tags and their relationships.

Supports Windows and Linux.

## Features

- Offline tag browsing and searching.
- Hierarchical categories and nested subgroups.
- Tag descriptions and relationships.
- Custom entries, descriptions, and connections.
- Manual index updates using the **Update Index** button.
- Index version history with protected snapshots.
- Import and export functionality.
- Light and dark themes.

## Installation

Download the latest installer from the repository's **Releases** section.

| Operating System | Package |
|---|---|
| Windows 11 (x64) | `.exe` |
| Fedora (x86_64) | `.rpm` |
| Debian / Ubuntu (x86_64) | `.deb` |

Linux installation:

**Fedora**
```bash
sudo dnf install ./danbooru-tag-indexer-4.0.0-1.x86_64.rpm
```

**Debian / Ubuntu**
```bash
sudo apt install ./danbooru-tag-indexer_4.0.0_amd64.deb
```

Linux requires glibc 2.39 or newer and WebKitGTK 4.1.

## Index Management

The application includes an offline index.

Index updates only run when manually initiated using **Update Index**. Updates can be paused, resumed, cancelled, or retried.

Custom entries and edits are preserved separately from index updates.

## Support

Support development through [Ko-fi](https://ko-fi.com/forbidden_fantasy).

## License & Credits

Licensed under the [Apache License 2.0](LICENSE).

Tag and wiki content is sourced from [Danbooru](https://danbooru.donmai.us/) and remains subject to its contributors' respective terms.

This project is independent and is not affiliated with Danbooru.