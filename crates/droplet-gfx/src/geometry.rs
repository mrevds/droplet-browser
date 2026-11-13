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

/// An integer pixel-aligned dirty rectangle for partial surface presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DamageRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl DamageRect {
    /// Construct a new damage rectangle from pixel origin and extents.
    #[inline]
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Construct a full-frame damage rectangle covering `[0, width) x [0, height)`.
    #[inline]
    pub const fn full(width: usize, height: usize) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Return whether the damage region covers zero pixels.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Clamp the damage rectangle to `[0, max_w) x [0, max_h)`.
    #[inline]
    pub fn clamp(self, max_w: usize, max_h: usize) -> Self {
        if self.is_empty() || self.x >= max_w || self.y >= max_h {
            return Self::default();
        }
        let right = self.x.saturating_add(self.width).min(max_w);
        let bottom = self.y.saturating_add(self.height).min(max_h);
        let width = right.saturating_sub(self.x);
        let height = bottom.saturating_sub(self.y);
        if width == 0 || height == 0 {
            Self::default()
        } else {
            Self {
                x: self.x,
                y: self.y,
                width,
                height,
            }
        }
    }

    /// Return the tight bounding rectangle enclosing both `self` and `other`.
    #[inline]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return if other.is_empty() {
                Self::default()
            } else {
                other
            };
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let bottom = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    /// Return the overlapping sub-rectangle of `self` and `other`, or an empty `DamageRect`.
    #[inline]
    pub fn intersect(self, other: Self) -> Self {
        if self.is_empty() || other.is_empty() {
            return Self::default();
        }
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self
            .x
            .saturating_add(self.width)
            .min(other.x.saturating_add(other.width));
        let bottom = self
            .y
            .saturating_add(self.height)
            .min(other.y.saturating_add(other.height));
        if x >= right || y >= bottom {
            Self::default()
        } else {
            Self {
                x,
                y,
                width: right - x,
                height: bottom - y,
            }
        }
    }

    /// Convert to a floating-point [`Rect`].
    #[inline]
    pub const fn to_rect(self) -> Rect {
        Rect {
            x: self.x as f32,
            y: self.y as f32,
            width: self.width as f32,
            height: self.height as f32,
        }
    }

    /// Snap a floating-point [`Rect`] outward to integer pixel bounds.
    #[inline]
    pub fn from_rect(rect: Rect) -> Self {
        if !rect.x.is_finite()
            || !rect.y.is_finite()
            || !rect.width.is_finite()
            || !rect.height.is_finite()
            || rect.width <= 0.0
            || rect.height <= 0.0
        {
            return Self::default();
        }
        let left = (rect.x as f64).max(0.0).floor();
        let top = (rect.y as f64).max(0.0).floor();
        let right = ((rect.x as f64) + (rect.width as f64)).max(0.0).ceil();
        let bottom = ((rect.y as f64) + (rect.height as f64)).max(0.0).ceil();
        if right <= left || bottom <= top {
            return Self::default();
        }
        Self {
            x: left as usize,
            y: top as usize,
            width: (right - left) as usize,
            height: (bottom - top) as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DamageRect, Rect};

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

    #[test]
    fn damage_rect_union_intersect_and_clamp() {
        let a = DamageRect::new(10, 20, 30, 40);
        let b = DamageRect::new(25, 10, 50, 25);
        assert!(!a.is_empty());
        assert!(DamageRect::default().is_empty());
        assert!(DamageRect::new(0, 0, 0, 10).is_empty());
        assert!(DamageRect::new(0, 0, 10, 0).is_empty());

        assert_eq!(a.union(b), DamageRect::new(10, 10, 65, 50));
        assert_eq!(a.union(DamageRect::default()), a);
        assert_eq!(DamageRect::default().union(b), b);
        assert_eq!(
            DamageRect::default().union(DamageRect::new(5, 5, 0, 4)),
            DamageRect::default()
        );

        assert_eq!(a.intersect(b), DamageRect::new(25, 20, 15, 15));
        assert_eq!(
            a.intersect(DamageRect::new(100, 100, 10, 10)),
            DamageRect::default()
        );
        assert_eq!(a.intersect(DamageRect::default()), DamageRect::default());

        assert_eq!(a.clamp(35, 50), DamageRect::new(10, 20, 25, 30));
        assert_eq!(a.clamp(10, 50), DamageRect::default());
        assert_eq!(a.clamp(50, 20), DamageRect::default());
        assert_eq!(DamageRect::full(800, 600), DamageRect::new(0, 0, 800, 600));
        assert_eq!(
            DamageRect::new(usize::MAX - 5, 0, 20, 10).clamp(usize::MAX, 100),
            DamageRect::new(usize::MAX - 5, 0, 5, 10)
        );
    }

    #[test]
    fn damage_rect_rect_conversions() {
        let r = Rect {
            x: -2.4,
            y: 3.2,
            width: 10.1,
            height: 4.5,
        };
        let d = DamageRect::from_rect(r);
        assert_eq!(d, DamageRect::new(0, 3, 8, 5));
        assert_eq!(
            d.to_rect(),
            Rect {
                x: 0.0,
                y: 3.0,
                width: 8.0,
                height: 5.0
            }
        );
        assert_eq!(
            DamageRect::from_rect(Rect {
                x: f32::NAN,
                y: 0.0,
                width: 10.0,
                height: 10.0
            }),
            DamageRect::default()
        );
    }
}
