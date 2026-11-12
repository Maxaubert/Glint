//! Shared building blocks for Glint: geometry, images, encoding, clipboard, config and paths.

pub mod clipboard;
pub mod config;
pub mod encode;
pub mod geom;
pub mod image;
pub mod naming;
pub mod paths;
pub mod system;

pub use geom::{Handle, Monitor, Rect};
pub use image::Image;
