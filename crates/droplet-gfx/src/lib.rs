//! Low-level graphics primitives (`Color`, `Rect`, `Canvas`) and rasterization helpers.

pub mod canvas;
pub mod color;
pub mod geometry;

pub use droplet_font as font;
pub use droplet_image as image;
pub use canvas::{draw_line_aa, Canvas};
pub use color::Color;
pub use geometry::Rect;
