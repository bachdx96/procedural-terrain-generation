use euclid::{Point3D, Vector3D};
use std::borrow::Borrow;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Triangle<T> {
    pub position: [Point3D<f32, T>; 3],
    pub id: [u64; 3],
}

#[derive(Debug)]
pub struct Mesh<T> {
    ids: Vec<u64>,
    vertex: Vec<Point3D<f32, T>>,
    faces: Vec<[usize; 3]>,
    normals: Option<Vec<Vector3D<f32, T>>>,
}

impl<T> Mesh<T> {
    pub fn from_triangles<I>(triangles: I) -> Self
    where
        I: IntoIterator,
        I::Item: Borrow<Triangle<T>>,
    {
        let mut index = 0;
        let mut id_to_index = HashMap::new();
        let mut vertex = vec![];
        let mut faces = vec![];
        let mut ids = vec![];
        for triangle in triangles.into_iter() {
            let triangle = triangle.borrow();
            let mut face_indices = triangle.id.iter().enumerate().map(|(i, &x)| {
                *id_to_index.entry(x).or_insert_with(|| {
                    let new_index = index;
                    index += 1;
                    vertex.push(triangle.position[i]);
                    ids.push(triangle.id[i]);
                    debug_assert_eq!(index, vertex.len());
                    new_index
                })
            });
            faces.push([
                face_indices.next().unwrap(),
                face_indices.next().unwrap(),
                face_indices.next().unwrap(),
            ]);
        }
        Mesh {
            ids,
            vertex,
            faces,
            normals: None,
        }
    }

    pub fn calculate_normals(&mut self) {
        let mut normals = vec![];
        let mut per_face_normals: HashMap<usize, Vec<_>> = HashMap::new();
        for face in &self.faces {
            let p0 = self.vertex[face[0]];
            let p1 = self.vertex[face[1]];
            let p2 = self.vertex[face[2]];
            let normal = (p1 - p0).cross(p0 - p2);
            for i in face.iter().take(3) {
                per_face_normals.entry(*i).or_default().push(normal);
            }
        }
        for i in 0..self.vertex.len() {
            normals.push(
                per_face_normals
                    .get(&i)
                    .unwrap()
                    .iter()
                    .fold(Vector3D::zero(), |acc, x| acc + x)
                    .normalize(),
            );
        }
        self.normals = Some(normals);
    }

    pub fn vertex(&self) -> &[Point3D<f32, T>] {
        &self.vertex
    }

    pub fn faces(&self) -> &[[usize; 3]] {
        &self.faces
    }

    pub fn normals(&self) -> &[Vector3D<f32, T>] {
        self.normals.as_ref().unwrap()
    }

    pub fn ids(&self) -> &[u64] {
        &self.ids
    }
}
