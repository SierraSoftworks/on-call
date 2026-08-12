//! Automatically compute a fair on-call schedule for your team.
//!
//! The scheduler works in three stages:
//!
//! 1. [`model::Problem`] resolves a [`config::Config`] and a date range into a
//!    flat, integer-indexed problem: the slots needing coverage, who may cover
//!    each one, and how much on-call time each person should end up with.
//! 2. [`objectives`] defines what makes one schedule better than another,
//!    as a two-tier [`model::Score`] where hard rule violations always dominate
//!    soft preferences.
//! 3. [`search`] builds an initial schedule greedily and then improves it with
//!    late-acceptance hill climbing, escaping local optima by periodically
//!    tearing out and rebuilding a section of the schedule.

#[macro_use]
mod macros;

pub mod config;
pub mod constraints;
pub mod model;
pub mod objectives;
pub mod output;
pub mod schedule;
pub mod search;
pub mod summary;
pub mod timerange;
