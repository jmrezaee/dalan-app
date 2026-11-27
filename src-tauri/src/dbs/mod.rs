//! Shared pieces of every database engine (replica/databases/spec.md section 3). D1 moved the transport here out of
//! `redis/`; the store, gate types and history follow as the SQL and MongoDB phases need them.

pub mod dumpfile;
pub mod dumpscan;
pub mod gate;
pub mod history;
pub mod jobs;
pub mod masking;
pub mod route;
pub mod runner;
pub mod saved;
pub mod schedule;
pub mod scheduler;
pub mod store;
pub mod transport;
pub mod url;

use serde::{Deserialize, Serialize};
use specta::Type;

/// The database engines Dalan can open. The names are the `engine` column of the `databases` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Redis,
    Postgres,
    Mysql,
    Mariadb,
    Mongodb,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Redis => "redis",
            Engine::Postgres => "postgres",
            Engine::Mysql => "mysql",
            Engine::Mariadb => "mariadb",
            Engine::Mongodb => "mongodb",
        }
    }

    pub fn parse(s: &str) -> Option<Engine> {
        [
            Engine::Redis,
            Engine::Postgres,
            Engine::Mysql,
            Engine::Mariadb,
            Engine::Mongodb,
        ]
        .into_iter()
        .find(|e| e.as_str() == s)
    }

    /// The name shown in messages.
    pub fn label(self) -> &'static str {
        match self {
            Engine::Redis => "Redis",
            Engine::Postgres => "PostgreSQL",
            Engine::Mysql => "MySQL",
            Engine::Mariadb => "MariaDB",
            Engine::Mongodb => "MongoDB",
        }
    }

    /// The port the engine listens on unless told otherwise.
    pub fn default_port(self) -> u16 {
        match self {
            Engine::Redis => 6379,
            Engine::Postgres => 5432,
            Engine::Mysql | Engine::Mariadb => 3306,
            Engine::Mongodb => 27017,
        }
    }
}

#[cfg(test)]
pub(crate) mod spike;
