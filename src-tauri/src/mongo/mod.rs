//! MongoDB inside Dalan (replica/databases/spec.md section 3, D8 onward). The `mongodb` crate can't take a stream
//! Dalan hands it (D0), so this is our own client: `OP_MSG` and BSON on the stream (direct, or an SSH `direct-tcpip`
//! channel), TLS wrapped around it as MongoDB wants, SCRAM sign-in, and commands as BSON documents.

pub mod conn;
pub mod docs;
