use crate::game::base::{Region, ScreenSpace, ViewSpace, WorldSpace};
use crate::gfx::Instance;
use euclid::{point2, vec3, Length, Point2D, Point3D, Transform3D, Vector3D};
use std::mem::size_of;
use std::sync::Arc;
use wgpu::util::StagingBelt;
use wgpu::*;

pub struct Camera {
    position: Point3D<f32, WorldSpace>,
    direction: Vector3D<f32, WorldSpace>,
    fov: f32,
    aspect_ratio: f32,
    near: f32,
    far: f32,
    buffer: Option<Arc<Buffer>>,
}

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod)]
#[repr(C)]
struct UniformData {
    view_matrix: [f32; 16],
    projection_matrix: [f32; 16],
}

impl Camera {
    pub fn new(
        position: Point3D<f32, WorldSpace>,
        direction: Vector3D<f32, WorldSpace>,
        fov: f32,
        aspect_ratio: f32,
        near: f32,
        far: f32,
    ) -> Self {
        Self {
            position,
            direction: direction.normalize(),
            fov,
            aspect_ratio,
            near,
            far,
            buffer: None,
        }
    }

    pub fn init(&mut self, instance: &Instance) {
        let device = instance.device();
        self.buffer = Some(Arc::new(device.create_buffer(&BufferDescriptor {
            label: Some("camera_uniform_buffer"),
            size: size_of::<UniformData>() as u64,
            usage: BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })));
    }

    pub fn position(&self) -> &Point3D<f32, WorldSpace> {
        &self.position
    }

    pub fn direction(&self) -> &Vector3D<f32, WorldSpace> {
        &self.direction
    }

    pub fn move_by(&mut self, offset: &Vector3D<f32, WorldSpace>) {
        self.position += *offset;
    }

    pub fn move_to(&mut self, new_position: &Point3D<f32, WorldSpace>) {
        self.position = *new_position;
    }

    pub fn look_at(&mut self, other: &Point3D<f32, WorldSpace>) {
        self.direction = (*other - self.position).normalize();
    }

    pub fn look_in_direction(&mut self, direction: &Vector3D<f32, WorldSpace>) {
        self.direction = direction.normalize();
    }

    pub fn fov_x(&self) -> f32 {
        (self.aspect_ratio * (self.fov / 2.0).tan()).atan() * 2.0
    }

    pub fn up(&self) -> Vector3D<f32, WorldSpace> {
        self.direction.cross(self.side()).normalize()
    }

    pub fn side(&self) -> Vector3D<f32, WorldSpace> {
        vec3(0.0, 0.0, 1.0).cross(self.direction).normalize()
    }

    pub fn point_from_distance(
        &self,
        point: Point2D<f32, ScreenSpace>,
        distance: Length<f32, WorldSpace>,
    ) -> Point3D<f32, WorldSpace> {
        self.position
            + self.side() * (self.fov_x() / 2.0).tan() * distance.get() * point.x
            + self.up() * (self.fov.tan() / 2.0) * distance.get() * point.y
            + self.direction * distance.get()
    }

    pub fn view_matrix(&self) -> Transform3D<f32, WorldSpace, ViewSpace> {
        let f = self.direction.normalize();
        let s = f.cross(self.up()).normalize();
        let u = s.cross(f);
        let eye = self.position.to_vector();
        Transform3D::new(
            s.x,
            u.x,
            -f.x,
            0.0,
            //
            s.y,
            u.y,
            -f.y,
            0.0,
            //
            s.z,
            u.z,
            -f.z,
            0.0,
            //
            -eye.dot(s),
            -eye.dot(u),
            eye.dot(f),
            1.0,
        )
    }

    pub fn projection_matrix(&self) -> Transform3D<f32, ViewSpace, ScreenSpace> {
        let f = (self.fov / 2.0).tan().recip();
        Transform3D::new(
            f / self.aspect_ratio,
            0.0,
            0.0,
            0.0,
            //
            0.0,
            f,
            0.0,
            0.0,
            //
            0.0,
            0.0,
            (self.far + self.near) / (self.near - self.far),
            -1.0,
            //
            0.0,
            0.0,
            (2.0 * self.far * self.near) / (self.near - self.far),
            0.0,
        )
    }

    pub fn update_buffer(
        &mut self,
        instance: &Instance,
        staging_belt: &mut StagingBelt,
        encoder: &mut CommandEncoder,
    ) {
        let device = instance.device();
        staging_belt
            .write_buffer(
                encoder,
                self.buffer.as_ref().unwrap(),
                0,
                BufferSize::new(size_of::<UniformData>() as _).unwrap(),
                device,
            )
            .copy_from_slice(bytemuck::bytes_of(&UniformData {
                view_matrix: self.view_matrix().to_array(),
                projection_matrix: self.projection_matrix().to_array(),
            }));
    }

    pub fn buffer(&self) -> Arc<Buffer> {
        self.buffer.as_ref().unwrap().clone()
    }

    pub fn lod_regions(&self, distance: f32, growth_factor: f32, count: usize) -> Vec<Region> {
        let mut regions = vec![];
        let y = if self.direction().z > 0.0 { -1.0 } else { 1.0 };
        regions.push(Region::new([
            self.point_from_distance(point2(-1.0, y), Length::new(distance))
                .xy(),
            self.point_from_distance(point2(1.0, y), Length::new(distance))
                .xy(),
            self.position().xy(),
        ]));
        let mut cummulate_growth = 1.0;
        for i in 1..count {
            let depth = growth_factor.powf(i as f32);
            let (p1, p2, p3, p4) = (
                self.point_from_distance(
                    point2(-1.0, y),
                    Length::new(distance * (cummulate_growth + depth)),
                ),
                self.point_from_distance(
                    point2(1.0, y),
                    Length::new(distance * (cummulate_growth + depth)),
                ),
                self.point_from_distance(point2(1.0, y), Length::new(distance * cummulate_growth)),
                self.point_from_distance(point2(-1.0, y), Length::new(distance * cummulate_growth)),
            );
            regions.push(Region::new([p1.xy(), p2.xy(), p3.xy(), p4.xy()]));
            cummulate_growth += depth;
        }
        regions
    }
}
