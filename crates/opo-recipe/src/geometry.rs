//! Geometry values produced by recipe operations.

use geo::MultiPolygon;
use opo_feel::{Foreign, Value};

use crate::Error;

/// The plane a 2D shape is drawn in (spec §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// Plan, normal +z: (u, v, w) → (u, v, w).
    Xy,
    /// Object face, normal +y: (u, v, w) → (u, w, v).
    Xz,
    /// Side, normal +x: (u, v, w) → (w, u, v).
    Yz,
    /// Local u/v plane of a swept profile (only meaningful inside a sweep).
    Profile,
}

impl Plane {
    /// The plane with this name (`xy`, `xz`, `yz`, `profile`).
    pub fn from_name(name: &str) -> Option<Plane> {
        Some(match name {
            "xy" => Plane::Xy,
            "xz" => Plane::Xz,
            "yz" => Plane::Yz,
            "profile" => Plane::Profile,
            _ => return None,
        })
    }

    /// Map plane coordinates (u, v) and a distance w along the normal to object coordinates.
    pub fn to_3d(self, u: f64, v: f64, w: f64) -> Result<[f64; 3], Error> {
        Ok(match self {
            Plane::Xy => [u, v, w],
            Plane::Xz => [u, w, v],
            Plane::Yz => [w, u, v],
            Plane::Profile => {
                return Err(Error::Op("a profile-plane region can only be swept".into()));
            }
        })
    }
}

/// A closed triangle mesh with outward-facing triangles (counter-clockwise seen from outside).
#[derive(Debug, Clone, PartialEq)]
pub struct Solid {
    /// Vertices.
    pub points: Vec<[f64; 3]>,
    /// Triangles as vertex indices.
    pub faces: Vec<[usize; 3]>,
}

impl Solid {
    /// Make a solid; if the signed volume is negative (a mirrored mapping), every face is
    /// flipped so the faces point outwards.
    pub fn new(points: Vec<[f64; 3]>, mut faces: Vec<[usize; 3]>) -> Self {
        let s = Solid {
            points,
            faces: Vec::new(),
        };
        if s.signed_volume_of(&faces) < 0.0 {
            for f in &mut faces {
                f.swap(1, 2);
            }
        }
        Solid { faces, ..s }
    }

    fn signed_volume_of(&self, faces: &[[usize; 3]]) -> f64 {
        faces
            .iter()
            .map(|[a, b, c]| {
                let (p, q, r) = (self.points[*a], self.points[*b], self.points[*c]);
                p[0] * (q[1] * r[2] - q[2] * r[1]) - p[1] * (q[0] * r[2] - q[2] * r[0])
                    + p[2] * (q[0] * r[1] - q[1] * r[0])
            })
            .sum::<f64>()
            / 6.0
    }

    /// Enclosed volume in m³ (absolute value of the signed volume).
    pub fn volume(&self) -> f64 {
        self.signed_volume_of(&self.faces).abs()
    }
}

/// A geometry value.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A 2D area (with holes) in a plane.
    Region {
        /// Plane.
        plane: Plane,
        /// Polygons in plane coordinates.
        shape: MultiPolygon<f64>,
    },
    /// A 2D polyline in a plane.
    Curve {
        /// Plane.
        plane: Plane,
        /// Points (u, v).
        points: Vec<[f64; 2]>,
    },
    /// A 3D polyline.
    Curve3 {
        /// Points (x, y, z).
        points: Vec<[f64; 3]>,
    },
    /// A closed mesh.
    Solid(Solid),
    /// A text label.
    Text {
        /// Plane.
        plane: Plane,
        /// Insertion point (u, v).
        at: [f64; 2],
        /// The text.
        value: String,
        /// Text height in metres.
        height: f64,
    },
}

/// A geometry value with what the recipe recorded about where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    /// The shape.
    pub shape: Shape,
    /// The `for_each` item that produced it, when that item is a context (for `each.<field>`
    /// material references).
    pub each: Option<Value>,
    /// 1-based position in its `for_each`.
    pub index: Option<usize>,
    /// Material resolved inside a nested object.
    pub material: Option<Value>,
    /// Id of the nested object it comes from.
    pub nested: Option<String>,
}

impl Geometry {
    /// A geometry value with no provenance.
    pub fn new(shape: Shape) -> Self {
        Geometry {
            shape,
            each: None,
            index: None,
            material: None,
            nested: None,
        }
    }

    /// Wrap as a FEEL value.
    pub fn into_value(self) -> Value {
        Value::Foreign(Foreign::new(self))
    }

    /// The solid, if this is one.
    pub fn solid(&self) -> Option<&Solid> {
        match &self.shape {
            Shape::Solid(s) => Some(s),
            _ => None,
        }
    }
}

/// Every geometry in a value: nested lists are flattened, `null` means nothing. Other values
/// are an error.
pub fn flatten(v: &Value) -> Result<Vec<Geometry>, Error> {
    let mut out = Vec::new();
    collect(v, &mut out)?;
    Ok(out)
}

fn collect(v: &Value, out: &mut Vec<Geometry>) -> Result<(), Error> {
    match v {
        Value::Null => Ok(()),
        Value::List(items) => items.iter().try_for_each(|x| collect(x, out)),
        Value::Foreign(f) => match f.downcast::<Geometry>() {
            Some(g) => {
                out.push(g.clone());
                Ok(())
            }
            None => Err(Error::Op("not a geometry value".into())),
        },
        other => Err(Error::Op(format!("expected geometry, got {other:?}"))),
    }
}

/// A list of geometry as one FEEL value.
pub fn list_value(items: Vec<Geometry>) -> Value {
    Value::List(items.into_iter().map(Geometry::into_value).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Solid {
        let p = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let f = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        Solid::new(p, f)
    }

    #[test]
    fn unit_cube_volume() {
        assert!((cube().volume() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn inverted_faces_are_flipped() {
        let c = cube();
        let inverted: Vec<_> = c.faces.iter().map(|f| [f[0], f[2], f[1]]).collect();
        let s = Solid::new(c.points.clone(), inverted);
        assert_eq!(s.faces, c.faces);
        assert!(s.signed_volume_of(&s.faces) > 0.0);
    }

    #[test]
    fn plane_mapping() {
        assert_eq!(Plane::Xz.to_3d(1.0, 2.0, 3.0).unwrap(), [1.0, 3.0, 2.0]);
        assert_eq!(Plane::Yz.to_3d(1.0, 2.0, 3.0).unwrap(), [3.0, 1.0, 2.0]);
        assert!(Plane::Profile.to_3d(0.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn flatten_lists_and_nulls() {
        let g = Geometry::new(Shape::Curve3 { points: vec![] });
        let v = Value::List(vec![
            Value::Null,
            Value::List(vec![g.clone().into_value()]),
            g.into_value(),
        ]);
        assert_eq!(flatten(&v).unwrap().len(), 2);
        assert!(flatten(&Value::Number(1.0)).is_err());
    }
}
