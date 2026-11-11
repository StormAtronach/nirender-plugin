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

pub(crate) use tiles::TileRead;
