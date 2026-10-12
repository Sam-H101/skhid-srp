// Vector math in the engine's conventions (`vostok/math_float4x4_inline.h`):
// row vectors, so a point transforms as `p * M`; a matrix's rows `i`, `j`, `k`
// are its axes and `c` its translation; `A * B` applies A first.

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

use vostok::math::float3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn from_array([x, y, z]: [f32; 3]) -> Self {
        Self::new(x, y, z)
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn squared_length(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        self.squared_length().sqrt()
    }

    /// The unit vector, or `fallback` for a (near) zero vector.
    pub fn normalized_or(self, fallback: Self) -> Self {
        let length = self.length();
        if length > 1e-12 {
            self * (1.0 / length)
        } else {
            fallback
        }
    }

    pub fn min(self, other: Self) -> Self {
        Self::new(
            self.x.min(other.x),
            self.y.min(other.y),
            self.z.min(other.z),
        )
    }

    pub fn max(self, other: Self) -> Self {
        Self::new(
            self.x.max(other.x),
            self.y.max(other.y),
            self.z.max(other.z),
        )
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }

    pub fn axis(self, index: usize) -> f32 {
        match index {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl From<float3> for Vec3 {
    fn from(value: float3) -> Self {
        Self::new(value.x, value.y, value.z)
    }
}

impl From<Vec3> for float3 {
    fn from(value: Vec3) -> Self {
        float3 {
            x: value.x,
            y: value.y,
            z: value.z,
        }
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, scale: f32) -> Self {
        Self::new(self.x * scale, self.y * scale, self.z * scale)
    }
}

/// An affine transform: rows `i`, `j`, `k` and translation `c`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub i: Vec3,
    pub j: Vec3,
    pub k: Vec3,
    pub c: Vec3,
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        i: Vec3::new(1.0, 0.0, 0.0),
        j: Vec3::new(0.0, 1.0, 0.0),
        k: Vec3::new(0.0, 0.0, 1.0),
        c: Vec3::ZERO,
    };

    pub fn translation(c: Vec3) -> Self {
        Self {
            c,
            ..Self::IDENTITY
        }
    }

    pub fn scale(s: Vec3) -> Self {
        Self {
            i: Vec3::new(s.x, 0.0, 0.0),
            j: Vec3::new(0.0, s.y, 0.0),
            k: Vec3::new(0.0, 0.0, s.z),
            c: Vec3::ZERO,
        }
    }

    /// `math::create_rotation( float3 const& angles )`.
    pub fn rotation_xyz(angles: Vec3) -> Self {
        let (xs, xc) = angles.x.sin_cos();
        let (ys, yc) = angles.y.sin_cos();
        let (zs, zc) = angles.z.sin_cos();
        Self {
            i: Vec3::new(yc * zc, -yc * zs, ys),
            j: Vec3::new(ys * xs * zc + xc * zs, -xs * ys * zs + xc * zc, -xs * yc),
            k: Vec3::new(-xc * ys * zc + xs * zs, xc * ys * zs + xs * zc, xc * yc),
            c: Vec3::ZERO,
        }
    }

    /// Angles that `rotation_xyz` turns back into this matrix's rotation
    /// (rows of unit length, at right angles).
    pub fn rotation_angles(&self) -> Vec3 {
        let y = self.i.z.clamp(-1.0, 1.0).asin();
        if y.cos() > 1e-4 {
            Vec3::new((-self.j.z).atan2(self.k.z), y, (-self.i.y).atan2(self.i.x))
        } else {
            // Straight up or down: only x + z (or x - z) is defined.
            Vec3::new(self.k.y.atan2(self.j.y), y, 0.0)
        }
    }

    /// `math::create_rotation_y`: the player's yaw. Forward (+z) turns to
    /// `(-sin, 0, cos)`.
    pub fn rotation_y(angle: f32) -> Self {
        Self::rotation_xyz(Vec3::new(0.0, angle, 0.0))
    }

    /// `math::create_rotation( float3 const& axis, float angle )`.
    pub fn rotation_axis(axis: Vec3, angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        let (x, y, z) = (axis.x, axis.y, axis.z);
        Self {
            i: Vec3::new(
                x * x + (1.0 - x * x) * c,
                x * y * t - z * s,
                x * z * t + y * s,
            ),
            j: Vec3::new(
                x * y * t + z * s,
                y * y + (1.0 - y * y) * c,
                y * z * t - x * s,
            ),
            k: Vec3::new(
                x * z * t - y * s,
                y * z * t + x * s,
                z * z + (1.0 - z * z) * c,
            ),
            c: Vec3::ZERO,
        }
    }

    /// Scale, then rotate, then translate: how the engine places objects.
    pub fn compose(scale: Vec3, angles: Vec3, position: Vec3) -> Self {
        Self::scale(scale) * Self::rotation_xyz(angles) * Self::translation(position)
    }

    pub fn transform_direction(&self, v: Vec3) -> Vec3 {
        self.i * v.x + self.j * v.y + self.k * v.z
    }

    pub fn transform_position(&self, v: Vec3) -> Vec3 {
        self.transform_direction(v) + self.c
    }

    /// The general affine inverse (handles scale); `None` if singular.
    pub fn inverse(&self) -> Option<Self> {
        let (a, b, c) = (self.i, self.j, self.k);
        let det = a.dot(b.cross(c));
        if det.abs() < 1e-20 {
            return None;
        }
        let inv_det = 1.0 / det;
        // Columns of the inverse of the 3x3 with rows a, b, c are the cross
        // products below; transpose them into rows.
        let col0 = b.cross(c) * inv_det;
        let col1 = c.cross(a) * inv_det;
        let col2 = a.cross(b) * inv_det;
        let rotation = Self {
            i: Vec3::new(col0.x, col1.x, col2.x),
            j: Vec3::new(col0.y, col1.y, col2.y),
            k: Vec3::new(col0.z, col1.z, col2.z),
            c: Vec3::ZERO,
        };
        Some(Self {
            c: -rotation.transform_direction(self.c),
            ..rotation
        })
    }
}

impl Mul for Mat4 {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self {
            i: other.transform_direction(self.i),
            j: other.transform_direction(self.j),
            k: other.transform_direction(self.k),
            c: other.transform_position(self.c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn yaw_turns_forward_like_the_engine() {
        let forward = Mat4::rotation_y(std::f32::consts::FRAC_PI_2).k;
        assert!(close(forward, Vec3::new(-1.0, 0.0, 0.0)), "{forward:?}");
    }

    #[test]
    fn composition_applies_left_first() {
        let m = Mat4::compose(
            Vec3::new(2.0, 2.0, 2.0),
            Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
        );
        // (0,0,1) scaled to (0,0,2), yawed 90 degrees to (-2,0,0), moved by +10 x.
        assert!(close(
            m.transform_position(Vec3::new(0.0, 0.0, 1.0)),
            Vec3::new(8.0, 0.0, 0.0)
        ));
        let back = m
            .inverse()
            .unwrap()
            .transform_position(Vec3::new(8.0, 0.0, 0.0));
        assert!(close(back, Vec3::new(0.0, 0.0, 1.0)), "{back:?}");
    }

    #[test]
    fn rotation_angles_rebuild_the_rotation() {
        let half_pi = std::f32::consts::FRAC_PI_2;
        for angles in [
            Vec3::new(0.3, -1.1, 2.5),
            Vec3::new(-2.9, 0.4, -0.7),
            Vec3::new(1.0, half_pi, 0.0),
            Vec3::new(0.2, -half_pi, 0.9),
        ] {
            let m = Mat4::rotation_xyz(angles);
            let rebuilt = Mat4::rotation_xyz(m.rotation_angles());
            for (a, b) in [(m.i, rebuilt.i), (m.j, rebuilt.j), (m.k, rebuilt.k)] {
                assert!((a - b).length() < 1e-3, "{angles:?}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn axis_rotation_keeps_the_axis_and_turns_the_rest() {
        let m = Mat4::rotation_axis(Vec3::Y, 0.7);
        assert!(close(m.transform_direction(Vec3::Y), Vec3::Y));
        let yaw = Mat4::rotation_y(0.7);
        let forward = Vec3::new(0.0, 0.0, 1.0);
        let turned = m.transform_direction(forward);
        assert!((turned.length() - 1.0).abs() < 1e-5);
        // Same plane as the yaw matrix; the direction of turn may differ.
        assert!((turned.y).abs() < 1e-6);
        assert!((turned.z - yaw.transform_direction(forward).z).abs() < 1e-5);
    }
}
