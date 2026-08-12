//! The problem model: what we are solving, and what a solution looks like.

pub mod assignment;
pub mod problem;
pub mod score;

pub use assignment::Assignment;
pub use problem::{Problem, SlotIdx};
pub use score::Score;
