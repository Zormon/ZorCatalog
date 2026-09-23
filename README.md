# ZorCatalog

Desktop app written in Tauri V2 for **cataloging drives and folders**: indexes your folder trees into a local SQLite database with image thumbnails, full-text search, catalog grouping, and portable backups.

You can browse what's on offline drives and folders without reconnecting them: inspect indexes, search by name, and preview image thumbnails.

Its ideal for data hoarders, archivists, and anyone who needs to catalog large collections of files and folders.

## Current Features

### Catalogs
- Recursively scan root folders into complete directory trees with total size, file count, and creation date.
- Cancelable scanning with live progress reporting (single scan at a time; clean rollback on failure/cancellation).
- Organize catalogs into single-level groups (name, color, collapse state) via drag-and-drop.

### Search
- Generates thumbnails of images (JPEG 256px for common formats up to 50MB).
- Full-text search of files and folders by name, filterable by scope (all, specific catalog, or group).

### Portable Backups (`.zcbak`)
- **Export**: Packs all catalogs, groups, nodes, and thumbnails into a single compressed portable archive.
- **Import**: Non-destructive merge (exact duplicates skipped, name collisions renamed with `(imported)` suffix, FTS index auto-rebuilt).

## Notes & Limitations
- Single-level grouping (no nested subgroups; one group per catalog).
- Indexes are point-in-time snapshots (re-scan required for disk updates).
- Unencrypted archives
- OS native file drag-and-drop into window disabled to support WebView2 HTML5 drag-and-drop on windows.
- Not tested on MacOS.

## Future Plans
- Tagging/metadata support for files and folders.
- Add support for encrypted portable backups.
- Add support for partial catalog exports (e.g., selected catalogs or groups).
- Add thumbnail generation for additional file types (e.g., PDF, video, audio).
- Navigation improvements (e.g., filtering, and sorting of files and folders).
