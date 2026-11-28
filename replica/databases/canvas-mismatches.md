# D12 to D17 boards vs the built screens (2026-11-28)

The boards DbRestore, DbCopy, DbJobs, DbCompare and DbBulk (UI/UX design canvas, "Databases" page, the row below the MongoDB boards) were drawn after the screens were built. I read each board and compared it with the built screens. The built screens are in `screens/` (the walk chapters 21 to 24, fake backend, 1280×800; refresh with `pnpm walk --only 21,22,23,24`, then copy `walk-report/img/2[1-4]-*.png` here):

| File | Screen |
| --- | --- |
| `21-01.png` | S48 What's inside |
| `21-02.png` | S48 target and review |
| `21-03.png` | S49 review with masking |
| `21-04.png` | S49 report |
| `22-01.png` … `22-04.png` | Save a backup as a job, Jobs with a schedule, missed runs, a failed run |
| `23-01.png` … `23-04.png` | Compare schemas, sync, data in a table, make target match |
| `24-01.png` … `24-03.png` | Bulk export, bulk import, backup check |

## Fixed in the app (the board was right)

- **DbCopy:** the masking count read "1 columns look personal". It now says "1 column looks personal".
- **DbRestore:** under the file's details, the line "Read from the file's first bytes, not its name." was added (the board's caption).
- **DbJobs, page header:** it now has the subtitle "Saved restores, copies and backups you can run again." and the note "Schedules run while Dalan is open and unlocked".
- **DbJobs, failure banner:** a banner names the job whose last run failed or was cut off, with **Open history**. The board shows it above the list. Before, there was only the dot on Jobs.
- **DbCompare:** the Sync button says how many changes it runs ("Sync 7 changes…"), as on the board.
- **Masking menu:** the option is "Shuffle across the table", because shuffling now mixes the whole table, not one batch.

## Board edits still needed (the built screen is right, or the board shows something Dalan can't know)

These are for whoever next edits the canvas. I can read the canvas but was asked not to change it.

### DbRestore

1. **Steps.** The board has six steps: File, What's inside, Target and options, Review, Progress, Result. The app has five: File, What's inside, Target and options, Review, Restore. In the app, progress and the result share the last step. Change the board's last two steps to one, "Restore".
2. **Row counts.** The board shows a row count per table and "≈ 12.4M rows". A dump's table of contents has no row counts (pg_restore's list doesn't carry them), so the app shows the kind and name of each object only. Remove the counts.
3. **Wording.** The board says "Needs attention"; the app says "Things that will need attention", the heading in `components.md`. Use the app's wording.
4. **Options.** "Data only" and "Schema only" are radio choices in the app, alongside "Schema and data", because they exclude each other. The board shows them as checkboxes.

### DbCopy

5. **Source and target.** On the board they are two cards with engine badges. In the app they are a target select ("name · engine") and a choice of "This database" or "Into a new database on that server". Redraw the target as the select with that choice.
6. **Object list.** The board shows a "related" tag and a row count on each table. The app ticks related tables with **Select related**, and shows the estimate in the review's "Rows (estimate)" column, not in the object list. Move the counts to the review table.
7. **Progress.** The board's line also names the tables waiting ("payments and the rest wait"). The app shows the current table, rows and rate. Remove the waiting list.

### DbJobs

8. **Layout.** The board shows the jobs as a table: Name, Kind, Route, Schedule, Next run, Last run, Saves to, Run. The app uses one row per job:
   - the name and kind;
   - the summary line;
   - the last run, with a missed-runs line when there are any;
   - the schedule line;
   - Run, Schedule…, History, rename and delete.
   
   History opens inside the row. A table can't open history inline, so redraw the board as rows.
9. **Next runs.** The board's schedule dialog shows a "Next runs" preview. The app shows the next run on the job's line instead (both keep rules, by count and by days, match the board).
10. **Unknown host key.** In the folder picker on the board, the box ends "Trust and continue". The app uses the usual key question, inline: Trust and connect, Connect once, Cancel. Use the same three choices.

### DbCompare

11. **Sides.** The board shows each side as one line ("PG app-staging · public"). The app shows two cards, each choosing a database, a snapshot or a dump file, plus schema selects and **Save a snapshot**. Redraw them as cards.
12. **Migration preview.** The board shows the migration script below the list of differences. In the app the script is shown by **Sync…**, in the review panel, and saved by **Copy as migration…**. Move the script into the review panel.
13. **Data compare summary.** The board shows four counts and "4 of 5 ranges skipped". The app says the same in one line: "Compared by id in 1.2 s · quick pass: 1 of 3 ranges the same". Use the app's sentence.

### DbBulk

14. **Export many tables.** The board's format choice is a segmented control (CSV, TSV, JSON lines, SQL inserts) and each table has a row count. In the app the format is a select and there are no counts. The masking rules apply to every chosen table with the column, and there is no "rules from Copy" option. Redraw to match.
15. **Import.** The board's "Preview each file" is the per-file **Mapping** or **CREATE TABLE** toggle in the app. The board's file-to-table arrows are the schema and table inputs in each file row.
16. **MongoDB import.** The board says "Insert or replace by email". The app's three choices are Insert, "Upsert by <field>" and "Replace the collection" (typed). The board's "Preview 20" is the sample of the first documents in the app.
17. **Check a backup.** The board's sources are "The backup I just made", "A file here" and "A file on a host". The app says "The backup just made", "A file on this computer" and "A file in a folder on a host (chosen when you press Check)". Use the app's wording.
18. **Import report.** The board's "Fix and run this file again" isn't in the app: a failed file is reported with the server's reason, and you run the import again with that file ticked. Remove the button, or keep it as a later idea.
