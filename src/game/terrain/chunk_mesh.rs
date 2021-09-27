use crate::game::base::{LocalSpace, WorldSpace};
use crate::game::mesh::Mesh;
use crate::game::terrain::chunk::Voxel;
use crate::gfx::Instance;
use euclid::{
    point2, point3, vec2, Box3D, Point2D, Point3D, Size2D, Size3D, Transform3D, UnknownUnit,
};
use futures::executor::block_on;
use futures::select;
use futures::FutureExt;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;

#[derive(Debug)]
pub struct VoxelFace {
    voxel_count: Size2D<u32, UnknownUnit>,
    voxels: Vec<f32>,
}

impl VoxelFace {
    pub fn new(voxel_count: Size2D<u32, UnknownUnit>, voxels: Vec<f32>) -> Self {
        Self {
            voxel_count,
            voxels,
        }
    }

    fn vertex(
        &self,
        voxel1: Point2D<u32, UnknownUnit>,
        voxel2: Point2D<u32, UnknownUnit>,
        isolevel: f32,
        stride: u32,
    ) -> Point2D<f32, LocalSpace> {
        let min = voxel1.min(voxel2);
        let max = voxel2.max(voxel1);
        assert!((max - min).lower_than([2, 2].into()).all());
        let min_stride = min / stride * stride;
        let max_stride = (max + vec2(stride - 1, stride - 1)) / stride * stride;
        assert!((max_stride - min_stride)
            .lower_than([stride + 1, stride + 1].into())
            .all());
        let stride1 = point2(
            if voxel1.x < voxel2.x {
                min_stride.x
            } else {
                max_stride.x
            },
            if voxel1.y < voxel2.y {
                min_stride.y
            } else {
                max_stride.y
            },
        );
        let stride2 = point2(
            if voxel2.x < voxel1.x {
                min_stride.x
            } else {
                max_stride.x
            },
            if voxel2.y < voxel1.y {
                min_stride.y
            } else {
                max_stride.y
            },
        );
        let v1 = self.voxels[self.point_to_index(stride1) as usize];
        let v2 = self.voxels[self.point_to_index(stride2) as usize];
        let p1 = stride1
            .to_vector()
            .to_f32()
            .component_div((self.voxel_count.to_vector() - vec2(1, 1)).to_f32())
            .clamp(vec2(0.0, 0.0), vec2(1.0, 1.0));
        let p2 = stride2
            .to_vector()
            .to_f32()
            .component_div((self.voxel_count.to_vector() - vec2(1, 1)).to_f32())
            .clamp(vec2(0.0, 0.0), vec2(1.0, 1.0));
        let min_v_fract = (voxel1.to_f32() - stride1.to_f32())
            .dot(stride2.to_f32() - stride1.to_f32())
            / (stride2.to_f32() - stride1.to_f32()).square_length();
        let max_v_fract = (voxel2.to_f32() - stride1.to_f32())
            .dot(stride2.to_f32() - stride1.to_f32())
            / (stride2.to_f32() - stride1.to_f32()).square_length();
        let p1 = p1 + (p2 - p1) * min_v_fract;
        let p2 = p1 + (p2 - p1) * max_v_fract;
        // if stride > 1 {
        //     println!("{} {}", min_v_fract, max_v_fract);
        // }
        let v1 = v1 + (v2 - v1) * min_v_fract;
        let v2 = v1 + (v2 - v1) * max_v_fract;
        let result = Self::vertex_lerp(
            isolevel.clamp(v1.min(v2), v1.max(v2)),
            p1.to_array().into(),
            p2.to_array().into(),
            v1,
            v2,
        );
        // if result.x > 1.0 || result.y > 1.0 {
        //     println!(
        //         "{:?} {:?} {:?} {:?} {:?}",
        //         p1,
        //         p2,
        //         v1 + (v2 - v1) * min_v_fract,
        //         v1 + (v2 - v1) * max_v_fract,
        //         result
        //     );
        // }
        result
    }

    fn point_to_index(&self, p: Point2D<u32, UnknownUnit>) -> u32 {
        p.x + self.voxel_count.width * p.y
    }

    fn vertex_lerp<T>(
        isolevel: f32,
        p1: Point2D<f32, T>,
        p2: Point2D<f32, T>,
        v1: f32,
        v2: f32,
    ) -> Point2D<f32, T> {
        if (isolevel - v1).abs() < 0.00001 {
            return p1;
        }
        if (isolevel - v2).abs() < 0.00001 {
            return p2;
        }
        let mu = (isolevel - v1) / (v2 - v1);
        p1 + (p2 - p1) * mu
    }
}

#[derive(Debug)]
pub struct EdgeVoxel {
    min_x: VoxelFace,
    max_x: VoxelFace,
    min_y: VoxelFace,
    max_y: VoxelFace,
}

impl EdgeVoxel {
    pub fn from_voxels(voxels: &[Voxel], size: Size3D<u32, UnknownUnit>) -> Self {
        let face_size = (size.height * size.depth) as usize;
        let mut min_x_voxels = Vec::with_capacity(face_size);
        let mut max_x_voxels = Vec::with_capacity(face_size);
        let mut min_y_voxels = Vec::with_capacity(face_size);
        let mut max_y_voxels = Vec::with_capacity(face_size);
        for z in 0..size.depth {
            for y in 0..size.height {
                min_x_voxels
                    .push(voxels[Self::voxel_point_to_index(point3(0, y, z), size) as usize].value);
                max_x_voxels.push(
                    voxels[Self::voxel_point_to_index(point3(size.width - 1, y, z), size) as usize]
                        .value,
                );
            }
        }

        for z in 0..size.depth {
            for x in 0..size.width {
                min_y_voxels
                    .push(voxels[Self::voxel_point_to_index(point3(x, 0, z), size) as usize].value);
                max_y_voxels.push(
                    voxels
                        [Self::voxel_point_to_index(point3(x, size.height - 1, z), size) as usize]
                        .value,
                );
            }
        }
        Self {
            min_x: VoxelFace::new([size.height, size.depth].into(), min_x_voxels),
            max_x: VoxelFace::new([size.height, size.depth].into(), max_x_voxels),
            min_y: VoxelFace::new([size.width, size.depth].into(), min_y_voxels),
            max_y: VoxelFace::new([size.width, size.depth].into(), max_y_voxels),
        }
    }

    fn voxel_point_to_index(p: Point3D<u32, UnknownUnit>, size: Size3D<u32, UnknownUnit>) -> u32 {
        p.x + size.width * (p.y + size.height * p.z)
    }
}

#[derive(Debug, Default)]
struct EdgeVertex {
    min_x: HashSet<usize>,
    max_x: HashSet<usize>,
    min_y: HashSet<usize>,
    max_y: HashSet<usize>,
}

type MapFuture = Pin<Box<dyn Future<Output = Result<(), BufferAsyncError>> + Send + Sync>>;

#[derive(PartialEq)]
pub enum MapStatus {
    Mapping,
    Mapped,
    Unmap,
}

pub struct ChunkMesh {
    bounds: Box3D<i32, WorldSpace>,
    voxel_count: Size3D<u32, UnknownUnit>,
    mesh: Mesh<LocalSpace>,
    vertex_buffer: Option<Buffer>,
    index_buffer: Option<Buffer>,
    uniform_buffer: Option<Buffer>,
    render_bundle: Option<RenderBundle>,
    edge_voxel: EdgeVoxel,
    edge_vertex: EdgeVertex,
    vertex_buffer_map_future: Option<MapFuture>,
}

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod)]
#[repr(C)]
pub struct VertexData {
    position: [f32; 4],
    normal: [f32; 4],
}

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod)]
#[repr(C)]
struct UniformData {
    world_matrix: [f32; 16],
}

impl ChunkMesh {
    pub fn new(
        bounds: Box3D<i32, WorldSpace>,
        mesh: Mesh<LocalSpace>,
        voxel_count: Size3D<u32, UnknownUnit>,
        edge_voxel: EdgeVoxel,
    ) -> Self {
        Self {
            bounds,
            mesh,
            voxel_count,
            vertex_buffer: None,
            index_buffer: None,
            uniform_buffer: None,
            render_bundle: None,
            edge_voxel,
            edge_vertex: Default::default(),
            vertex_buffer_map_future: None,
        }
    }

    fn transformation_matrix(&self) -> Transform3D<f32, LocalSpace, WorldSpace> {
        let bounds = self.bounds.to_f32();
        Transform3D::scale(bounds.width(), bounds.height(), bounds.depth())
            .then_translate(bounds.min.to_vector())
    }

    pub fn create_render_resources(
        &mut self,
        instance: &Instance,
        pipeline: &RenderPipeline,
        bind_group_layout: &BindGroupLayout,
        camera_uniform_buffer: &Buffer,
        target_format: TextureFormat,
    ) {
        if self.vertex_buffer.is_some() || self.uniform_buffer.is_some() {
            return;
        }
        for (i, id) in self.mesh.ids().iter().enumerate() {
            let [i1, i2]: [u32; 2] = unsafe { std::mem::transmute(*id) };
            let p1 = self.voxel_index_to_point(i1);
            let p2 = self.voxel_index_to_point(i2);
            if p1.x == 0 && p2.x == 0 {
                self.edge_vertex.min_x.insert(i);
            } else if p1.y == 0 && p2.y == 0 {
                self.edge_vertex.min_y.insert(i);
            } else if p1.x == self.voxel_count.width - 1 && p2.x == self.voxel_count.width - 1 {
                self.edge_vertex.max_x.insert(i);
            } else if p1.y == self.voxel_count.height - 1 && p2.y == self.voxel_count.height - 1 {
                self.edge_vertex.max_y.insert(i);
            }
        }
        let device = instance.device();
        let vertex_buffer_data: Vec<_> = self
            .mesh
            .vertex()
            .iter()
            .zip(self.mesh.normals().iter())
            .map(|(v, n)| VertexData {
                position: [v.x, v.y, v.z, 1.0],
                normal: [n.x, n.y, n.z, 1.0],
            })
            .collect();
        let index_buffer_data: Vec<_> = self
            .mesh
            .faces()
            .iter()
            .flat_map(|x| x.map(|x| x as u32))
            .collect();
        self.vertex_buffer = Some(device.create_buffer_init(&BufferInitDescriptor {
            label: Some("chunk_mesh_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertex_buffer_data),
            usage: BufferUsages::VERTEX | BufferUsages::MAP_WRITE,
        }));
        self.index_buffer = Some(device.create_buffer_init(&BufferInitDescriptor {
            label: Some("chunk_mesh_index_buffer"),
            contents: bytemuck::cast_slice(&index_buffer_data),
            usage: BufferUsages::INDEX,
        }));
        self.uniform_buffer = Some(device.create_buffer_init(&BufferInitDescriptor {
            label: Some("chunk_mesh_uniform_buffer"),
            contents: bytemuck::bytes_of(&UniformData {
                world_matrix: self.transformation_matrix().to_array(),
            }),
            usage: BufferUsages::UNIFORM,
        }));
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: self.uniform_buffer.as_ref().unwrap(),
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: camera_uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
            label: Some("chunk_mesh_bind_group"),
            layout: bind_group_layout,
        });
        let mut encoder = device.create_render_bundle_encoder(&RenderBundleEncoderDescriptor {
            label: Some("chunk_mesh_render_bundle_encoder"),
            color_formats: &[target_format],
            depth_stencil: Some(RenderBundleDepthStencil {
                format: TextureFormat::Depth32Float,
                depth_read_only: false,
                stencil_read_only: false,
            }),
            sample_count: 1,
        });
        encoder.set_bind_group(0, &bind_group, &[]);
        encoder.set_vertex_buffer(0, self.vertex_buffer.as_ref().unwrap().slice(..));
        encoder.set_index_buffer(
            self.index_buffer.as_ref().unwrap().slice(..),
            IndexFormat::Uint32,
        );
        encoder.set_pipeline(pipeline);
        encoder.draw_indexed(0..index_buffer_data.len() as u32, 0, 0..1);
        self.render_bundle = Some(encoder.finish(&RenderBundleDescriptor {
            label: Some("chunk_mesh_render_bundle"),
        }));
    }

    pub fn render_bundle(&self) -> Option<&RenderBundle> {
        self.render_bundle.as_ref()
    }

    pub fn map_vertex_buffer(&mut self) {
        if self.vertex_buffer_map_future.is_none() {
            let buffer_slice = self.vertex_buffer.as_ref().unwrap().slice(..);
            self.vertex_buffer_map_future = Some(Box::pin(buffer_slice.map_async(MapMode::Write)));
        }
    }

    pub fn map_vertex_buffer_status(&mut self) -> MapStatus {
        if self.vertex_buffer_map_future.is_none() {
            return MapStatus::Unmap;
        }
        let mut future = self.vertex_buffer_map_future.as_mut().unwrap().fuse();
        block_on(async {
            select! {
                _ = future => MapStatus::Mapped,
                default  => MapStatus::Mapping,
                complete =>  MapStatus::Mapped
            }
        })
    }

    pub fn stitch_edges(
        &self,
        min_x_stride: u32,
        max_x_stride: u32,
        min_y_stride: u32,
        max_y_stride: u32,
    ) {
        {
            let buffer_slice = self.vertex_buffer.as_ref().unwrap().slice(..);
            block_on(buffer_slice.map_async(MapMode::Write)).unwrap();
            let mut raw_buffer = &mut *buffer_slice.get_mapped_range_mut();
            let buffer = bytemuck::cast_slice_mut::<_, VertexData>(&mut raw_buffer);
            let normals = self.mesh.normals();
            let ids = self.mesh.ids();
            for i in &self.edge_vertex.min_x {
                let [i1, i2]: [u32; 2] = unsafe { std::mem::transmute(ids[*i]) };
                let voxel_point_1 = self.voxel_index_to_point(i1);
                let voxel_point_2 = self.voxel_index_to_point(i2);
                let p = self.edge_voxel.min_x.vertex(
                    voxel_point_1.yz(),
                    voxel_point_2.yz(),
                    0.5,
                    min_x_stride,
                );
                // println!("{:?}", p);
                let n = normals[*i];
                buffer[*i] = VertexData {
                    position: [0.0, p.x, p.y, 1.0],
                    normal: [n.x, n.y, n.z, 0.0],
                }
            }
            for i in &self.edge_vertex.max_x {
                let [i1, i2]: [u32; 2] = unsafe { std::mem::transmute(ids[*i]) };
                let voxel_point_1 = self.voxel_index_to_point(i1);
                let voxel_point_2 = self.voxel_index_to_point(i2);
                let p = self.edge_voxel.max_x.vertex(
                    voxel_point_1.yz(),
                    voxel_point_2.yz(),
                    0.5,
                    max_x_stride,
                );
                let n = normals[*i];
                buffer[*i] = VertexData {
                    position: [1.0, p.x, p.y, 1.0],
                    normal: [n.x, n.y, n.z, 0.0],
                }
            }
            for i in &self.edge_vertex.min_y {
                let [i1, i2]: [u32; 2] = unsafe { std::mem::transmute(ids[*i]) };
                let voxel_point_1 = self.voxel_index_to_point(i1);
                let voxel_point_2 = self.voxel_index_to_point(i2);
                let p = self.edge_voxel.min_y.vertex(
                    voxel_point_1.xz(),
                    voxel_point_2.xz(),
                    0.5,
                    min_y_stride,
                );
                let n = normals[*i];
                buffer[*i] = VertexData {
                    position: [p.x, 0.0, p.y, 1.0],
                    normal: [n.x, n.y, n.z, 0.0],
                }
            }
            for i in &self.edge_vertex.max_y {
                let [i1, i2]: [u32; 2] = unsafe { std::mem::transmute(ids[*i]) };
                let voxel_point_1 = self.voxel_index_to_point(i1);
                let voxel_point_2 = self.voxel_index_to_point(i2);
                let p = self.edge_voxel.max_y.vertex(
                    voxel_point_1.xz(),
                    voxel_point_2.xz(),
                    0.5,
                    max_y_stride,
                );
                let n = normals[*i];
                buffer[*i] = VertexData {
                    position: [p.x, 1.0, p.y, 1.0],
                    normal: [n.x, n.y, n.z, 0.0],
                }
            }
        }
        self.vertex_buffer.as_ref().unwrap().unmap();
    }

    fn voxel_index_to_point(&self, i: u32) -> Point3D<u32, UnknownUnit> {
        point3(
            i % self.voxel_count.width,
            (i / self.voxel_count.width) % self.voxel_count.height,
            i / (self.voxel_count.width * self.voxel_count.height),
        )
    }
}

impl From<std::sync::RwLock<ChunkMesh>> for ChunkMesh {
    fn from(item: std::sync::RwLock<ChunkMesh>) -> ChunkMesh {
        item.into_inner().unwrap()
    }
}
