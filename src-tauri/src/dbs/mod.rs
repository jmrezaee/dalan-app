//! Shared pieces of every database engine (replica/databases/spec.md section 3). D1 moved the transport here out of
//! `redis/`; the store, gate types and history follow as the SQL and MongoDB phases need them.

pub mod transport;

#[cfg(test)]
mod spike;
