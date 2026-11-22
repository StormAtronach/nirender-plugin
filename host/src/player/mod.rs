//! The player's methods by concern, as further `impl Player` blocks (the struct stays in
//! lib.rs).

mod draws;
mod feed;
mod frame;
mod items;
pub(crate) use items::LATE_BEFORE;
mod report;
mod split;
mod tiles;
mod world_starts;

pub(crate) use draws::ParticleAnswers;
pub(crate) use tiles::{TileLoss, TileRead};
pub(crate) use world_starts::FirstWorldFrames;
