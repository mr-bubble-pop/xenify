//! Word documents (.docx, Office Open XML).

mod omml;
mod read;
mod write;

pub use read::read;
pub use write::{write, write_with_pages};
