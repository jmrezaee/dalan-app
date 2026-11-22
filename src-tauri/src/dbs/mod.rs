//! Shared pieces of the SQL and MongoDB databases (replica/databases/spec.md section 3). D0 only adds the driver
//! spike; D1 moves the transport, store, gate types and history here out of `redis/`.

#[cfg(test)]
mod spike;
