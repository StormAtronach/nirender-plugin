//! The player's methods by concern, as further `impl Player` blocks (the struct stays in
//! lib.rs).

mod draws;
mod feed;
mod frame;
mod items;
mod report;
mod tiles;

pub(crate) use tiles::TileRead;
