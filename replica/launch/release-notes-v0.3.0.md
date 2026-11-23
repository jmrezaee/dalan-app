# Dalan v0.3.0: PostgreSQL, MySQL, MariaDB and MongoDB

Dalan now opens SQL and MongoDB databases next to Redis, reached directly or through a saved SSH host (nothing listens on your computer). Every engine has the same write modes: Read-only, Confirm writes and Open. A change is checked in Rust: reads run, changes show what they will do (with how many rows or documents they reach) and ask first, dangerous ones ask you to type the database's name.

## SQL (PostgreSQL, MySQL, MariaDB)
- Tables with paging, filters and sorting; a query editor with a result per statement, history and a stop button.
- Manual transactions, reviewed grid edits, structure changes, plans (Explain), a relationship diagram.
- Activity: sessions, cancel and end.
- Export to CSV, JSON or SQL; CSV import with a column mapping; backups with the engine's own tools, run on the host.

## MongoDB
- Collections with filter, projection, sort and collation; list, table and JSON views; editing with exact types.
- An aggregation builder (stages you can switch off, the output after each, export as code, views, `$out`/`$merge` after a confirmation).
- Schema from a sample, indexes with how often they are used, validation rules.
- A shell in the familiar `db.collection` syntax (no JavaScript), export, and `mongodump`/`mongorestore` backups.

## Also
- Settings > Databases: rows a SQL query shows and its timeout. The Redis settings there now persist (they were not saved before).

## Known limits
- Not yet: MongoDB SRV names, SCRAM-SHA-1 and X.509 sign-in; importing JSON/CSV into a MongoDB collection; inserting rows in the SQL grid; the confirm-over-N-rows setting.
- Mac builds are not signed until the Apple Developer account exists (right-click, Open).
