//! Hand-rolled 2-D geometry: points and a projective (perspective) transform.
//!
//! Coordinates follow the workspace convention `(x, y)` = (column, row) with the
//! origin at the top-left corner and `y` growing downward, matching image row
//! order.

/// A point in image space (`x` = column, `y` = row), sub-pixel precision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Column coordinate (grows rightward).
    pub x: f64,
    /// Row coordinate (grows downward).
    pub y: f64,
}

impl Point {
    /// Creates a point at `(x, y)`.
    #[inline]
    pub fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    /// Euclidean distance to `other`.
    #[inline]
    pub fn distance(self, other: Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Squared Euclidean distance to `other` (no `sqrt`).
    #[inline]
    pub fn distance_sq(self, other: Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

/// A 3x3 projective transform mapping 2-D points through homogeneous
/// coordinates. Built from four point correspondences (or three, degenerating
/// to affine). The formulas follow the classic square-to-quadrilateral
/// construction; the transform is only defined up to scale, so an adjoint is a
/// valid inverse.
#[derive(Clone, Copy, Debug)]
pub struct PerspectiveTransform {
    a11: f64,
    a21: f64,
    a31: f64,
    a12: f64,
    a22: f64,
    a32: f64,
    a13: f64,
    a23: f64,
    a33: f64,
}

impl PerspectiveTransform {
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn new9(
        a11: f64,
        a21: f64,
        a31: f64,
        a12: f64,
        a22: f64,
        a32: f64,
        a13: f64,
        a23: f64,
        a33: f64,
    ) -> PerspectiveTransform {
        PerspectiveTransform {
            a11,
            a21,
            a31,
            a12,
            a22,
            a32,
            a13,
            a23,
            a33,
        }
    }

    /// Maps a single point through the transform.
    #[inline]
    pub fn map(&self, x: f64, y: f64) -> Point {
        let denom = self.a13 * x + self.a23 * y + self.a33;
        Point {
            x: (self.a11 * x + self.a21 * y + self.a31) / denom,
            y: (self.a12 * x + self.a22 * y + self.a32) / denom,
        }
    }

    /// Transform taking the unit square corners `(0,0),(1,0),(1,1),(0,1)` onto
    /// the four given points, in that order.
    #[allow(clippy::too_many_arguments)]
    fn square_to_quad(
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        x3: f64,
        y3: f64,
    ) -> PerspectiveTransform {
        let dx3 = x0 - x1 + x2 - x3;
        let dy3 = y0 - y1 + y2 - y3;
        if dx3.abs() < f64::EPSILON && dy3.abs() < f64::EPSILON {
            // Affine special case (the mapped quad is a parallelogram).
            PerspectiveTransform::new9(x1 - x0, x2 - x1, x0, y1 - y0, y2 - y1, y0, 0.0, 0.0, 1.0)
        } else {
            let dx1 = x1 - x2;
            let dx2 = x3 - x2;
            let dy1 = y1 - y2;
            let dy2 = y3 - y2;
            let denominator = dx1 * dy2 - dx2 * dy1;
            let a13 = (dx3 * dy2 - dx2 * dy3) / denominator;
            let a23 = (dx1 * dy3 - dx3 * dy1) / denominator;
            PerspectiveTransform::new9(
                x1 - x0 + a13 * x1,
                x3 - x0 + a23 * x3,
                x0,
                y1 - y0 + a13 * y1,
                y3 - y0 + a23 * y3,
                y0,
                a13,
                a23,
                1.0,
            )
        }
    }

    /// Adjoint (classical adjugate) — a valid inverse up to a scale factor,
    /// which is irrelevant for projective transforms.
    fn adjoint(&self) -> PerspectiveTransform {
        PerspectiveTransform::new9(
            self.a22 * self.a33 - self.a23 * self.a32,
            self.a23 * self.a31 - self.a21 * self.a33,
            self.a21 * self.a32 - self.a22 * self.a31,
            self.a13 * self.a32 - self.a12 * self.a33,
            self.a11 * self.a33 - self.a13 * self.a31,
            self.a12 * self.a31 - self.a11 * self.a32,
            self.a12 * self.a23 - self.a13 * self.a22,
            self.a13 * self.a21 - self.a11 * self.a23,
            self.a11 * self.a22 - self.a12 * self.a21,
        )
    }

    /// Matrix product `self * other` (apply `other` first, then `self`).
    fn times(&self, o: &PerspectiveTransform) -> PerspectiveTransform {
        PerspectiveTransform::new9(
            self.a11 * o.a11 + self.a21 * o.a12 + self.a31 * o.a13,
            self.a11 * o.a21 + self.a21 * o.a22 + self.a31 * o.a23,
            self.a11 * o.a31 + self.a21 * o.a32 + self.a31 * o.a33,
            self.a12 * o.a11 + self.a22 * o.a12 + self.a32 * o.a13,
            self.a12 * o.a21 + self.a22 * o.a22 + self.a32 * o.a23,
            self.a12 * o.a31 + self.a22 * o.a32 + self.a32 * o.a33,
            self.a13 * o.a11 + self.a23 * o.a12 + self.a33 * o.a13,
            self.a13 * o.a21 + self.a23 * o.a22 + self.a33 * o.a23,
            self.a13 * o.a31 + self.a23 * o.a32 + self.a33 * o.a33,
        )
    }

    /// Transform mapping the four `src` corners onto the four `dst` corners, in
    /// matching order. Applying it to a `src`-space point yields the
    /// corresponding `dst`-space point.
    pub fn quad_to_quad(src: [Point; 4], dst: [Point; 4]) -> PerspectiveTransform {
        let s_to_q = PerspectiveTransform::square_to_quad(
            dst[0].x, dst[0].y, dst[1].x, dst[1].y, dst[2].x, dst[2].y, dst[3].x, dst[3].y,
        );
        let q_to_s = PerspectiveTransform::square_to_quad(
            src[0].x, src[0].y, src[1].x, src[1].y, src[2].x, src[2].y, src[3].x, src[3].y,
        )
        .adjoint();
        s_to_q.times(&q_to_s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    #[test]
    fn identity_quad_maps_corners() {
        let sq = [
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(0.0, 1.0),
        ];
        let t = PerspectiveTransform::quad_to_quad(sq, sq);
        assert!(approx(t.map(0.5, 0.5), Point::new(0.5, 0.5)));
        assert!(approx(t.map(0.0, 0.0), Point::new(0.0, 0.0)));
    }

    #[test]
    fn maps_named_corners_exactly() {
        let src = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        // A genuinely projective destination quad.
        let dst = [
            Point::new(5.0, 4.0),
            Point::new(90.0, 12.0),
            Point::new(80.0, 95.0),
            Point::new(12.0, 70.0),
        ];
        let t = PerspectiveTransform::quad_to_quad(src, dst);
        for i in 0..4 {
            assert!(approx(t.map(src[i].x, src[i].y), dst[i]), "corner {i}");
        }
    }

    #[test]
    fn roundtrip_forward_then_inverse() {
        let src = [
            Point::new(3.5, 3.5),
            Point::new(21.5, 3.5),
            Point::new(21.5, 21.5),
            Point::new(3.5, 21.5),
        ];
        let dst = [
            Point::new(40.0, 38.0),
            Point::new(200.0, 55.0),
            Point::new(190.0, 210.0),
            Point::new(48.0, 195.0),
        ];
        let fwd = PerspectiveTransform::quad_to_quad(src, dst);
        let inv = PerspectiveTransform::quad_to_quad(dst, src);
        let p = fwd.map(10.0, 14.0);
        let back = inv.map(p.x, p.y);
        assert!(approx(back, Point::new(10.0, 14.0)));
    }
}
