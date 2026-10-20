//! A durable on-disk buffer: a FIFO of accepted items that survives agent
//! restarts until each one is resolved. Built on [`crate::queue::Queue`].

pub mod dir;
pub mod errors;
pub mod job;

pub use self::dir::{Buffer, BufferOptions};
pub use self::errors::{BufferErr, BufferFullErr, ItemTooLargeErr, SerializeItemErr};
pub use self::job::{BufferJob, Item};
