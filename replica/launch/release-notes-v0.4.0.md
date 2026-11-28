# Dalan v0.4.0: restore any dump, copy between databases, jobs and compare

You can now restore a dump made anywhere, copy tables or collections from one database to another, save those as jobs (with masking and a schedule), and compare two schemas or two tables. As before, every change is checked in Rust first: you see what it will do, and a production database asks you to type its name.

## Restore any dump
- Dalan reads the format from the file's own bytes: PostgreSQL (custom, tar, folder, plain SQL, pg_dumpall), mysqldump/mariadb-dump and mongodump archives or folders. Compressed files work too (gzip, zstd, bzip2, xz).
- Before anything runs, Dalan shows what's inside and what will need attention. You pick the target, an existing database or a new one, and the tables to bring.
- A plain SQL dump with a command that would run a program is refused.
- Restore can be stopped, and in one transaction nothing changes.
- Drop a dump file from Finder (or Explorer) onto a database tab to start.

## Copy between databases
- Tables or collections can go from one database to another, on the same engine or between PostgreSQL and MySQL/MariaDB.
- Between engines, you see and can change each column's type; columns that lose something are marked.
- Rows are checked on both sides. A table that fails can run again on its own.

## Jobs, masking and schedules
- Save a copy, restore or backup as a job and run it again from Vault > Jobs.
- Masking changes personal columns between the source and the target: fake names and emails, hashes that keep joins, empty, shuffle and more. The real values never reach the target, and a production-to-staging copy without masking reminds you.
- MongoDB exports can be masked too.
- Backups can run on a schedule into a folder on a saved host, and keep the last N files or the last N days.
  - Runs missed while Dalan was closed or locked run once at the next unlock.
  - A failed run shows a dot and a banner on Jobs.
  - Quitting during a run asks first, and a run cut off is recorded and runs again.
- Every backup is read back when it ends. "Check a backup…" restores it in a scratch database, counts the rows and removes it again.

## Compare
- Two schemas (a database, a saved snapshot or a dump): the differences, a sync script that runs in one transaction, or a migration file to keep.
- Two tables by key: rows only in one side and rows that differ. A quick pass skips ranges that match. You can make the target match.

## Bulk
- Export many tables at once into a folder or a .zip, with a manifest and the tables' DDL.
- Import a folder or a .zip back: each file is matched to a table, and missing tables are created from the manifest's types.
- Import JSON lines, JSON arrays, extended JSON or typed CSV into a MongoDB collection (insert, upsert or replace).

## Also
- Dragging tabs, SFTP files, hosts and snippets works the same, now with an Escape to cancel.
- Upgrading: your vault is updated to the new version on first start; nothing is removed. Keep v0.3.0's vault backup if you might go back, because v0.3.0 can't open an updated vault.

## Known limits
- Backups and restores through a host run the database tools on that host, so it needs the PostgreSQL, MySQL or MongoDB client tools.
- Shuffle masking isn't available for MongoDB fields.
- Mac builds are not signed until the Apple Developer account exists (right-click, Open). Windows may show a SmartScreen warning.
