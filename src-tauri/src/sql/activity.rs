//! Activity (S41, F25): who is connected and what they run, what waits for whom, and a few server-wide views. Every
//! query only reads. Cancelling a query or ending a session is a change to somebody else's work, so it goes through
//! the gate (`commands::sql::sql_session_*`), never from here.

use serde::Serialize;
use specta::Type;

use super::pg::{map_error, PgConn};
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ActivityView {
    Sessions,
    Locks,
    Slow,
    Tables,
    Roles,
    Replication,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ActivityTiles {
    pub connections: u32,
    pub max_connections: u32,
    pub active: u32,
    pub waiting: u32,
    /// Commits plus rollbacks so far; the window turns two readings into a rate.
    pub transactions_total: f64,
    pub cache_hit_pct: Option<f64>,
    pub replicas: u32,
    pub replication_lag_s: Option<f64>,
    pub db_bytes: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionRow {
    pub pid: i32,
    pub user: String,
    pub application: String,
    pub state: String,
    /// Seconds in this state.
    pub age_s: f64,
    pub wait: String,
    pub query: String,
    pub blocked_by: Vec<i32>,
    /// One of Dalan's own connections: it can be seen but not cancelled or ended from here.
    pub mine: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ChainLink {
    pub pid: i32,
    pub user: String,
    pub state: String,
    pub age_s: f64,
    pub query: String,
}

/// One session that blocks others, and everyone stuck behind it (directly or through another waiter).
#[derive(Debug, Clone, Serialize, Type)]
pub struct BlockingChain {
    pub root: ChainLink,
    pub waiting: Vec<ChainLink>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ActivityTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct Activity {
    pub tiles: ActivityTiles,
    pub sessions: Vec<SessionRow>,
    pub chains: Vec<BlockingChain>,
    /// For every view but Sessions.
    pub table: Option<ActivityTable>,
}

fn link(s: &SessionRow) -> ChainLink {
    ChainLink {
        pid: s.pid,
        user: s.user.clone(),
        state: s.state.clone(),
        age_s: s.age_s,
        query: s.query.clone(),
    }
}

/// Groups the waiters under the session at the head of each chain. A cycle (a deadlock the server has not broken
/// yet) has no head: its first member stands in. Pure, so it is tested.
pub fn chains(sessions: &[SessionRow]) -> Vec<BlockingChain> {
    let by_pid = |p: i32| sessions.iter().find(|s| s.pid == p);
    let root_of = |s: &SessionRow| {
        let mut cur = s;
        for _ in 0..sessions.len() + 1 {
            match cur.blocked_by.iter().find_map(|b| by_pid(*b)) {
                Some(next) if next.pid != s.pid => cur = next,
                _ => break,
            }
        }
        cur.pid
    };
    let mut out: Vec<BlockingChain> = Vec::new();
    for s in sessions.iter().filter(|s| !s.blocked_by.is_empty()) {
        let root = root_of(s);
        let Some(r) = by_pid(root) else { continue };
        if r.pid == s.pid {
            continue;
        }
        match out.iter_mut().find(|c| c.root.pid == root) {
            Some(c) => c.waiting.push(link(s)),
            None => out.push(BlockingChain {
                root: link(r),
                waiting: vec![link(s)],
            }),
        }
    }
    out
}

impl PgConn {
    pub async fn activity(&self, view: ActivityView) -> AppResult<Activity> {
        self.read_only(async |c| {
            let t = c
                .query_one(
                    "select (select count(*) from pg_stat_activity where backend_type = 'client backend')::int,
                            current_setting('max_connections')::int,
                            (select count(*) from pg_stat_activity where state = 'active' and backend_type = 'client backend' and pid <> pg_backend_pid())::int,
                            (select count(*) from pg_stat_activity where wait_event_type = 'Lock')::int,
                            coalesce((select xact_commit + xact_rollback from pg_stat_database where datname = current_database()), 0)::float8,
                            (select case when blks_hit + blks_read = 0 then null else blks_hit::float8 * 100 / (blks_hit + blks_read) end from pg_stat_database where datname = current_database()),
                            pg_database_size(current_database())::float8,
                            (select count(*) from pg_stat_replication)::int,
                            (select extract(epoch from max(replay_lag)) from pg_stat_replication)::float8",
                    &[],
                )
                .await?;
            let tiles = ActivityTiles {
                connections: t.get::<_, i32>(0).max(0) as u32,
                max_connections: t.get::<_, i32>(1).max(0) as u32,
                active: t.get::<_, i32>(2).max(0) as u32,
                waiting: t.get::<_, i32>(3).max(0) as u32,
                transactions_total: t.get(4),
                cache_hit_pct: t.get(5),
                db_bytes: t.get(6),
                replicas: t.get::<_, i32>(7).max(0) as u32,
                replication_lag_s: t.get(8),
            };
            let mut sessions = Vec::new();
            let mut table = None;
            if view == ActivityView::Sessions {
                let rows = c
                    .query(
                        "select a.pid, coalesce(a.usename, '')::text, coalesce(a.application_name, '')::text, coalesce(a.state, '')::text,
                                coalesce(extract(epoch from now() - a.state_change), 0)::float8,
                                (coalesce(a.wait_event_type, '') || case when a.wait_event is not null then ' ' || a.wait_event else '' end)::text,
                                left(a.query, 2000)::text, pg_blocking_pids(a.pid), coalesce(a.application_name = 'Dalan', false)
                         from pg_stat_activity a where a.backend_type = 'client backend' and a.pid <> pg_backend_pid()
                         order by (a.state = 'active') desc, a.state_change limit 500",
                        &[],
                    )
                    .await?;
                sessions = rows
                    .iter()
                    .map(|r| SessionRow { pid: r.get(0), user: r.get(1), application: r.get(2), state: r.get(3), age_s: r.get(4), wait: r.get(5), query: r.get(6), blocked_by: r.get(7), mine: r.get(8) })
                    .collect();
            }
            else {
                let (columns, sql): (&[&str], &str) = match view {
                    ActivityView::Locks => (&["PID", "User", "Type", "Relation", "Mode", "Granted"],
                        "select l.pid::text, coalesce(a.usename, '')::text, l.locktype::text, coalesce(c.relname, '')::text, l.mode::text, l.granted::text
                         from pg_locks l left join pg_class c on c.oid = l.relation left join pg_stat_activity a on a.pid = l.pid
                         where l.locktype in ('relation', 'tuple', 'transactionid', 'advisory') and l.pid <> pg_backend_pid() order by l.granted, l.pid limit 500"),
                    ActivityView::Slow => (&["PID", "User", "Running for", "Query"],
                        "select pid::text, coalesce(usename, '')::text, to_char(now() - query_start, 'HH24:MI:SS'), left(query, 400)::text
                         from pg_stat_activity where state = 'active' and backend_type = 'client backend' and pid <> pg_backend_pid() order by query_start limit 50"),
                    ActivityView::Tables => (&["Table", "Rows", "Size", "Seq scans", "Index scans", "Dead rows", "Last vacuum"],
                        "select schemaname || '.' || relname, n_live_tup::text, pg_size_pretty(pg_total_relation_size(relid)), seq_scan::text, coalesce(idx_scan, 0)::text, n_dead_tup::text,
                                coalesce(to_char(greatest(last_vacuum, last_autovacuum), 'YYYY-MM-DD HH24:MI'), 'never')
                         from pg_stat_user_tables order by pg_total_relation_size(relid) desc limit 100"),
                    ActivityView::Roles => (&["Role", "Rights", "Login", "Connection limit", "Sessions"],
                        "select rolname::text, case when rolsuper then 'superuser' else '' end, case when rolcanlogin then 'login' else 'no login' end, rolconnlimit::text,
                                (select count(*) from pg_stat_activity where usename = rolname)::text from pg_roles where rolname !~ '^pg_' order by 1"),
                    _ => (&["Replica", "Address", "State", "Sync", "Replay lag"],
                        "select coalesce(application_name, '')::text, coalesce(client_addr::text, ''), coalesce(state, '')::text, coalesce(sync_state, '')::text, coalesce(replay_lag::text, '') from pg_stat_replication"),
                };
                let rows = c.query(sql, &[]).await?;
                let n = columns.len();
                table = Some(ActivityTable {
                    columns: columns.iter().map(|s| s.to_string()).collect(),
                    rows: rows.iter().map(|r| (0..n).map(|i| r.get::<_, Option<String>>(i).unwrap_or_default()).collect()).collect(),
                });
            }
            let chains = chains(&sessions);
            Ok(Activity { tiles, sessions, chains, table })
        })
        .await
    }

    /// Cancels the query (`end` false) or ends the session (`end` true) of another client. Dalan's own connections
    /// and anything that is not a client backend are never touched. `None`: there was no such session.
    pub async fn signal_session(
        &self,
        pid: i32,
        end: bool,
        started: &str,
    ) -> AppResult<Option<bool>> {
        let f = if end {
            "pg_terminate_backend"
        } else {
            "pg_cancel_backend"
        };
        let sql = format!(
            "select case when exists (select 1 from pg_stat_activity where pid = $1 and backend_type = 'client backend' and coalesce(application_name, '') <> 'Dalan' and pid <> pg_backend_pid() and backend_start::text = $2)
                         then {f}($1) end"
        );
        let _scope = self.scope().await;
        let row = self
            .client
            .query_one(&sql, &[&pid, &started])
            .await
            .map_err(|e| map_error(&e))?;
        Ok(row.get(0))
    }

    /// A session as the confirmation shows it, with when it started: a pid reused by another session within the
    /// confirmation's two minutes has another start time and is left alone.
    pub async fn session_info(
        &self,
        pid: i32,
    ) -> AppResult<Option<(String, String, String, String)>> {
        let _scope = self.scope().await;
        let row = self
            .client
            .query_opt(
                "select coalesce(usename, '')::text, coalesce(state, '')::text, left(query, 300)::text, backend_start::text from pg_stat_activity
                 where pid = $1 and backend_type = 'client backend' and coalesce(application_name, '') <> 'Dalan' and pid <> pg_backend_pid()",
                &[&pid],
            )
            .await
            .map_err(|e| map_error(&e))?;
        Ok(row.map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(pid: i32, blocked_by: &[i32]) -> SessionRow {
        SessionRow {
            pid,
            user: "u".into(),
            application: String::new(),
            state: "active".into(),
            age_s: 1.0,
            wait: String::new(),
            query: format!("q{pid}"),
            blocked_by: blocked_by.to_vec(),
            mine: false,
        }
    }

    #[test]
    fn waiters_group_under_the_head_of_their_chain() {
        // 1 blocks 2 and 3; 3 blocks 4; 9 blocks nobody.
        let all = [s(1, &[]), s(2, &[1]), s(3, &[1]), s(4, &[3]), s(9, &[])];
        let c = chains(&all);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].root.pid, 1);
        assert_eq!(
            c[0].waiting.iter().map(|w| w.pid).collect::<Vec<_>>(),
            [2, 3, 4]
        );
    }

    #[test]
    fn a_deadlock_cycle_and_a_blocker_we_cannot_see_do_not_loop() {
        let cyc = [s(1, &[2]), s(2, &[1])];
        let c = chains(&cyc);
        assert!(c.len() <= 2 && c.iter().all(|x| !x.waiting.is_empty()));
        // The blocker (pid 7) is not among the sessions: no chain head to show.
        assert!(chains(&[s(2, &[7])]).is_empty());
    }
}
