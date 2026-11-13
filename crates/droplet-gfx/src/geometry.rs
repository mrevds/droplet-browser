//! 2D geometric primitives.

/// An axis-aligned rectangle in floating-point pixel coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Right edge X coordinate (`x + width`).
    #[inline]
    pub const fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Bottom edge Y coordinate (`y + height`).
    #[inline]
    pub const fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Return whether point `(px, py)` falls within `[x, right()) x [y, bottom())`.
    #[inline]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }
}

#[cfg(test)]
mod tests {
    use super::Rect;

    #[test]
    fn rect_edges_and_contains() {
        let r = Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        assert!((r.right() - 40.0).abs() < 1e-6);
        assert!((r.bottom() - 60.0).abs() < 1e-6);

        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(25.0, 45.0));
        assert!(!r.contains(40.0, 20.0));
        assert!(!r.contains(10.0, 60.0));
        assert!(!r.contains(9.9, 20.0));

        let empty = Rect::default();
        assert!(!empty.contains(0.0, 0.0));
    }
}
