use super::SHADER_WORKGROUP_SIZE;
use crate::game::base::WorldSpace;
use crate::game::mesh::Triangle;
use crate::gfx::Instance;
use euclid::{size3, Box3D, Point3D, Size3D, UnknownUnit};
use futures::executor::block_on;
use std::mem::size_of;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod, Default)]
#[repr(C)]
struct GenerateVoxelInfo {
    voxel_count: [u32; 3],
    lod: u32,
    min: [f32; 3],
    _pad1: u32,
    max: [f32; 3],
    _pad2: u32,
}

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod)]
#[repr(C)]
struct GenerateTriangleInfo {
    cell_count: [u32; 3],
    isolevel: f32,
}

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod)]
#[repr(C)]
pub struct Voxel {
    pub value: f32,
}

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod)]
#[repr(C)]
struct ComputeTriangle {
    position: [[f32; 4]; 3],
    id: [[u32; 2]; 3],
    _pad: u64,
}

pub struct Chunk {
    bounds: Box3D<i32, WorldSpace>,
    level: u32,
    voxel_count: Size3D<u32, UnknownUnit>,
    staging_voxel_buffer: Option<Buffer>,
    voxel_buffer: Option<Buffer>,
    staging_triangle_buffer: Option<Buffer>,
    triangle_buffer: Option<Buffer>,
}

impl Chunk {
    pub fn new(
        bounds: Box3D<i32, WorldSpace>,
        level: u32,
        voxel_count: Size3D<u32, UnknownUnit>,
    ) -> Self {
        Self {
            bounds,
            level,
            voxel_count,
            voxel_buffer: None,
            staging_voxel_buffer: None,
            triangle_buffer: None,
            staging_triangle_buffer: None,
        }
    }

    fn voxel_buffer_size(&self) -> u64 {
        self.total_voxel_count() as u64 * size_of::<Voxel>() as u64
    }

    fn triangle_buffer_size(&self) -> u64 {
        8 + self.total_cell_count() as u64 * 5 * size_of::<ComputeTriangle>() as u64
    }

    #[profiling::function]
    fn create_staging_voxel_buffer(&mut self, instance: &Instance) {
        if self.staging_voxel_buffer.is_some() {
            return;
        }
        let device = instance.device();

        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("chunk_staging_voxel_buffer"),
            size: self.voxel_buffer_size(),
            mapped_at_creation: false,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        });
        self.staging_voxel_buffer = Some(buffer);
    }

    #[profiling::function]
    fn create_staging_triangle_buffer(&mut self, instance: &Instance) {
        if self.staging_triangle_buffer.is_some() {
            return;
        }
        let device = instance.device();

        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("chunk_staging_triangle_buffer"),
            size: self.triangle_buffer_size(),
            mapped_at_creation: false,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        });
        self.staging_triangle_buffer = Some(buffer);
    }

    #[profiling::function]
    fn create_voxel_buffer(&mut self, instance: &Instance) {
        let device = instance.device();
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("chunk_voxel_buffer"),
            size: self.voxel_buffer_size(),
            mapped_at_creation: false,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        });
        self.voxel_buffer = Some(buffer);
    }

    #[profiling::function]
    fn create_triangle_buffer(&mut self, instance: &Instance) {
        let device = instance.device();
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("chunk_triangle_buffer"),
            size: self.triangle_buffer_size(),
            mapped_at_creation: false,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        });
        self.triangle_buffer = Some(buffer);
    }

    #[profiling::function]
    pub fn generate_voxel(
        &mut self,
        instance: &Instance,
        encoder: &mut CommandEncoder,
        generate_voxel_pipeline: &ComputePipeline,
        copy_to_staging: bool,
    ) {
        self.create_voxel_buffer(instance);
        if copy_to_staging {
            self.create_staging_voxel_buffer(instance);
        } else {
            self.staging_voxel_buffer = None;
        }
        let device = instance.device();
        let bounds = self.bounds.to_f32();
        let data = GenerateVoxelInfo {
            voxel_count: self.voxel_count.to_array(),
            lod: self.level,
            min: bounds.min.to_array(),
            max: bounds.max.to_array(),
            ..Default::default()
        };
        let uniform_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("chunk_voxel_uniform_buffer"),
            contents: bytemuck::bytes_of(&data),
            usage: BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: self.voxel_buffer.as_ref().unwrap(),
                        offset: 0,
                        size: None,
                    }),
                },
            ],
            label: Some("chunk_voxel_bind_group"),
            layout: &generate_voxel_pipeline.get_bind_group_layout(0),
        });
        {
            let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("chunk_voxel_compute_pass"),
            });
            // Divide number of vertex per side by local size then round up
            let group_count_x =
                (self.voxel_count.width + SHADER_WORKGROUP_SIZE - 1) / SHADER_WORKGROUP_SIZE;
            let group_count_y =
                (self.voxel_count.height + SHADER_WORKGROUP_SIZE - 1) / SHADER_WORKGROUP_SIZE;
            let group_count_z =
                (self.voxel_count.depth + SHADER_WORKGROUP_SIZE - 1) / SHADER_WORKGROUP_SIZE;
            compute_pass.set_pipeline(generate_voxel_pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            compute_pass.dispatch(group_count_x, group_count_y, group_count_z);
        }
        if copy_to_staging {
            encoder.copy_buffer_to_buffer(
                self.voxel_buffer.as_ref().unwrap(),
                0,
                self.staging_voxel_buffer.as_ref().unwrap(),
                0,
                self.voxel_buffer_size(),
            );
        }
    }

    #[profiling::function]
    pub fn generate_triangle(
        &mut self,
        instance: &Instance,
        encoder: &mut CommandEncoder,
        generate_triangle_pipeline: &ComputePipeline,
        copy_to_staging: bool,
        isolevel: f32,
    ) {
        self.create_triangle_buffer(instance);
        if copy_to_staging {
            self.create_staging_triangle_buffer(instance);
        } else {
            self.staging_voxel_buffer = None;
        }
        let device = instance.device();
        let data = GenerateTriangleInfo {
            cell_count: (self.voxel_count - size3(1, 1, 1)).to_array(),
            isolevel,
        };

        let uniform_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("chunk_triangle_uniform_buffer"),
            contents: bytemuck::bytes_of(&data),
            usage: BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: self.voxel_buffer.as_ref().unwrap(),
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: self.triangle_buffer.as_ref().unwrap(),
                        offset: 0,
                        size: None,
                    }),
                },
            ],
            label: Some("chunk_triangle_bind_group"),
            layout: &generate_triangle_pipeline.get_bind_group_layout(0),
        });
        {
            let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("chunk_triangle_compute_pass"),
            });
            // Divide number of (vertex per side - 1) by local size then round up
            let group_count_x =
                (self.voxel_count.width + SHADER_WORKGROUP_SIZE - 1) / SHADER_WORKGROUP_SIZE;
            let group_count_y =
                (self.voxel_count.height + SHADER_WORKGROUP_SIZE - 1) / SHADER_WORKGROUP_SIZE;
            let group_count_z =
                (self.voxel_count.depth + SHADER_WORKGROUP_SIZE - 1) / SHADER_WORKGROUP_SIZE;
            compute_pass.set_pipeline(generate_triangle_pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            compute_pass.dispatch(group_count_x, group_count_y, group_count_z);
        }
        if copy_to_staging {
            encoder.copy_buffer_to_buffer(
                self.triangle_buffer.as_ref().unwrap(),
                0,
                self.staging_triangle_buffer.as_ref().unwrap(),
                0,
                self.triangle_buffer_size(),
            );
        }
    }

    // WARNING: Do not call this on main thread, it will block until
    // GPU device is polled
    pub fn map_voxel_buffer(&mut self) {
        debug_assert!(self.staging_voxel_buffer.is_some());
        let buffer_slice = self.staging_voxel_buffer.as_ref().unwrap().slice(..);
        block_on(buffer_slice.map_async(MapMode::Read)).unwrap();
    }

    pub fn unmap_voxel_buffer(&mut self) {
        debug_assert!(self.staging_voxel_buffer.is_some());
        self.staging_voxel_buffer.as_ref().unwrap().unmap();
    }

    // WARNING: Do not call this on main thread, it will block until
    // GPU device is polled
    #[profiling::function]
    pub fn map_triangle_buffer(&mut self) {
        debug_assert!(self.staging_triangle_buffer.is_some());
        let buffer_slice = self.staging_triangle_buffer.as_ref().unwrap().slice(..);
        block_on(buffer_slice.map_async(MapMode::Read)).unwrap();
    }

    pub fn unmap_triangle_buffer(&mut self) {
        debug_assert!(self.staging_triangle_buffer.is_some());
        self.staging_triangle_buffer.as_ref().unwrap().unmap();
    }

    pub fn get_mapped_voxel_buffer(&self) -> Vec<Voxel> {
        let buffer_slice = self.staging_voxel_buffer.as_ref().unwrap().slice(..);
        let data = buffer_slice.get_mapped_range();
        bytemuck::cast_slice(&data).to_vec()
    }

    #[profiling::function]
    pub fn get_mapped_triangle_buffer<T>(&self) -> Vec<Triangle<T>>
    where
        T: Send,
    {
        let buffer_slice = self.staging_triangle_buffer.as_ref().unwrap().slice(..);
        let data = buffer_slice.get_mapped_range();
        let triangle_count: u32 = *bytemuck::from_bytes(&data[..4]);
        if triangle_count == 0 {
            vec![]
        } else {
            let compute_triangles: &[ComputeTriangle] = bytemuck::cast_slice(
                &data[16..16 + size_of::<ComputeTriangle>() * triangle_count as usize],
            );
            compute_triangles
                .iter()
                .map(|t| Triangle {
                    position: t.position.map(|x| Point3D::from([x[0], x[1], x[2]])),
                    id: unsafe { std::mem::transmute(t.id) },
                })
                .collect()
        }
    }

    fn total_voxel_count(&self) -> u32 {
        self.voxel_count.volume()
    }

    fn total_cell_count(&self) -> u32 {
        (self.voxel_count - size3(1, 1, 1)).volume()
    }

    pub fn bounds(&self) -> Box3D<i32, WorldSpace> {
        self.bounds
    }

    pub fn voxel_buffer(&self) -> Option<&Buffer> {
        self.voxel_buffer.as_ref()
    }

    pub fn voxel_count(&self) -> Size3D<u32, UnknownUnit> {
        self.voxel_count
    }

    pub fn triangle_buffer(&self) -> Option<&Buffer> {
        self.triangle_buffer.as_ref()
    }

    pub fn clear_triangle_buffer(&mut self) {
        self.triangle_buffer = None
    }
}
