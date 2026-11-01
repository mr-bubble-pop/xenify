//! Email for Xenify Chat, without a user interface: accounts (passwords in the
//! system keyring), IMAP, sending, the local mail store and keeping it in
//! sync. The desktop app shows it; this crate does the work.

pub mod account;
pub mod auth;
pub mod graph;
pub mod imap;
pub mod message;
pub mod send;
pub mod store;
pub mod sync;
