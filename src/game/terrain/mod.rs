mod cache;
mod chunk;
mod chunk_mesh;
mod tree;

use crate::game::base::WorldSpace;
use crate::game::mesh::Mesh;
use crate::{game::base::Region, gfx::Instance};
use cache::Cache;
use chunk::Chunk;
use chunk_mesh::{ChunkMesh, EdgeVoxel, MapStatus, VertexData};
use crossbeam_deque::{Injector, Worker};
use euclid::size3;
use euclid::Box3D;
use euclid::Point3D;
use parking_lot::{RwLock, RwLockReadGuard};
use std::mem::size_of;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use tree::Tree;
use wgpu::*;

// Keep in sync with shader
const SHADER_WORKGROUP_SIZE: u32 = 8;

#[derive(Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub struct ChunkCacheKey {
    pub bounds: Box3D<i32, WorldSpace>,
    pub level: u32,
}

pub struct TerrainRegion {
    pub region: Region,
    pub level: u32,
}

#[derive(Debug, Copy, Clone)]
struct StitchStride {
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
}

enum TerrainTask {
    GenerateChunk(ChunkCacheKey),
    WriteChunk(ChunkCacheKey, Chunk),
    InvalidateTriangle,
    RegenerateTriangle(ChunkCacheKey),
    GenerateMesh(ChunkCacheKey),
    WriteMesh(ChunkCacheKey, ChunkMesh),
    GenerateMeshResouces(ChunkCacheKey),
    StitchMesh(ChunkCacheKey, StitchStride),
}

pub struct Terrain {
    terrain_data: Arc<TerrainData>,
    injector: Arc<Injector<TerrainTask>>,
    thread_handles: Vec<JoinHandle<()>>,
    condvar: Arc<Condvar>,
    guard: Arc<Mutex<bool>>,
}

impl Terrain {
    pub fn new() -> Self {
        Self {
            terrain_data: Arc::new(TerrainData::new()),
            injector: Arc::new(Injector::new()),
            thread_handles: vec![],
            condvar: Arc::new(Condvar::new()),
            guard: Arc::new(false.into()),
        }
    }

    pub fn init(
        &mut self,
        instance: Arc<Instance>,
        target_format: TextureFormat,
        camera_buffer: Arc<Buffer>,
        isolevel: f32,
    ) {
        Arc::get_mut(&mut self.terrain_data)
            .unwrap()
            .init(&instance, target_format);
        self.terrain_data.set_isolevel(isolevel);
        let mut worker_queues = (0..1)
            .map(|_| Worker::new_fifo())
            .collect::<Vec<Worker<TerrainTask>>>();
        let stealers = worker_queues
            .iter()
            .map(|x| x.stealer())
            .collect::<Vec<_>>();
        for (i, local) in worker_queues.drain(..).enumerate() {
            let guard = self.guard.clone();
            let condvar = self.condvar.clone();
            let global = self.injector.clone();
            let stealers = stealers
                .iter()
                .enumerate()
                .filter_map(|(j, x)| if i == j { None } else { Some(x.clone()) })
                .collect::<Vec<_>>();
            let terrain_data = self.terrain_data.clone();
            let instance = instance.clone();
            let camera_buffer = camera_buffer.clone();

            let t = std::thread::spawn(move || {
                profiling::register_thread!();
                loop {
                    loop {
                        let task = local.pop().or_else(|| {
                            // Otherwise, we need to look for a task elsewhere.
                            std::iter::repeat_with(|| {
                                // Try stealing a batch of tasks from the global queue.
                                global
                                    .steal_batch_and_pop(&local)
                                    // Or try stealing a task from one of the other threads.
                                    .or_else(|| stealers.iter().map(|s| s.steal()).collect())
                            })
                            // Loop while no task was stolen and any steal operation needs to be retried.
                            .find(|s| !s.is_retry())
                            // Extract the stolen task, if there is one.
                            .and_then(|s| s.success())
                        });
                        if task.is_none() {
                            break;
                        }
                        let mut next_task = task;
                        while let Some(t) = next_task {
                            next_task = match t {
                                TerrainTask::GenerateChunk(key) => {
                                    terrain_data.generate_chunk(&instance, &key)
                                }
                                TerrainTask::WriteChunk(key, chunk) => {
                                    terrain_data.write_chunk(&key, chunk)
                                }
                                TerrainTask::GenerateMesh(key) => terrain_data.generate_mesh(&key),
                                TerrainTask::WriteMesh(key, mesh) => {
                                    terrain_data.write_mesh(&key, mesh)
                                }
                                TerrainTask::GenerateMeshResouces(key) => terrain_data
                                    .generate_mesh_resources(&instance, &camera_buffer, &key),
                                TerrainTask::RegenerateTriangle(key) => {
                                    terrain_data.regenerate_triangle(&instance, &key)
                                }
                                TerrainTask::InvalidateTriangle => {
                                    terrain_data.invalidate_triangle()
                                }
                                TerrainTask::StitchMesh(key, stride) => {
                                    terrain_data.stitch_mesh(&key, &stride)
                                }
                            }
                        }
                    }
                    let mut done = guard.lock().unwrap();
                    done = condvar.wait(done).unwrap();
                    if *done {
                        break;
                    }
                }
            });
            self.thread_handles.push(t);
        }
        let instance = instance.clone();
        let guard = self.guard.clone();
        let condvar = self.condvar.clone();
        // let t = std::thread::spawn(move || {
        //     profiling::register_thread!();
        //     loop {
        //         instance.device().poll(Maintain::Wait);
        //         // let mut done = guard.lock().unwrap();
        //         // done = condvar.wait(done).unwrap();
        //         // if *done {
        //         //     break;
        //         // }
        //     }
        // });
        // self.thread_handles.push(t);
    }

    #[profiling::function]
    pub fn update_terrain(&self, position: &Point3D<f32, WorldSpace>, regions: &[TerrainRegion]) {
        {
            let mut tree = self.terrain_data.tree.write();
            for region in regions {
                tree.ensure_node_in_region(&region.region);
                tree.set_level_in_region(&region.region, region.level);
            }
            tree.rebuild_tree();
        }
        let tree = self.terrain_data.tree.read();
        let mut keys = vec![];
        for node in tree.leaf_intersect_regions_iter(
            regions
                .iter()
                .map(|x| x.region.clone())
                .collect::<Vec<_>>()
                .as_slice(),
        ) {
            let bounds = node.bounds();
            let level = node.level();
            let key = ChunkCacheKey { bounds, level };
            keys.push(key);
        }
        keys.sort_by(|a, b| {
            b.bounds
                .center()
                .to_f32()
                .distance_to(*position)
                .partial_cmp(&a.bounds.center().to_f32().distance_to(*position))
                .unwrap()
        });
        self.terrain_data.update_last_accessed(&keys);
        for (i, key) in keys.iter().rev().enumerate() {
            self.injector.push(TerrainTask::GenerateChunk(*key));
            self.condvar.notify_one();
            // let mut stride = StitchStride {
            //     min_x: 1,
            //     max_x: 1,
            //     min_y: 1,
            //     max_y: 1,
            // };
            // for other in keys.iter().rev().skip(i + 1) {
            //     if other.bounds.max.x == key.bounds.min.x
            //         && ((other.bounds.min.y >= key.bounds.min.y
            //             && other.bounds.min.y < key.bounds.max.y)
            //             || (other.bounds.max.y <= key.bounds.max.y
            //                 && other.bounds.max.y > key.bounds.min.y))
            //     {
            //         stride.min_x = 2u32
            //             .pow((key.level as i32 - other.level as i32).abs() as u32)
            //             .max(stride.min_x)
            //     }
            //     if other.bounds.min.x == key.bounds.max.x
            //         && ((other.bounds.min.y >= key.bounds.min.y
            //             && other.bounds.min.y < key.bounds.max.y)
            //             || (other.bounds.max.y <= key.bounds.max.y
            //                 && other.bounds.max.y > key.bounds.min.y))
            //     {
            //         stride.max_x = 2u32
            //             .pow((key.level as i32 - other.level as i32).abs() as u32)
            //             .max(stride.max_x)
            //     }
            //     if other.bounds.max.y == key.bounds.min.y
            //         && ((other.bounds.min.x >= key.bounds.min.x
            //             && other.bounds.min.x < key.bounds.max.x)
            //             || (other.bounds.max.x <= key.bounds.max.x
            //                 && other.bounds.max.x > key.bounds.min.x))
            //     {
            //         stride.min_y = 2u32
            //             .pow((key.level as i32 - other.level as i32).abs() as u32)
            //             .max(stride.min_y)
            //     }
            //     if other.bounds.min.y == key.bounds.max.y
            //         && ((other.bounds.min.x >= key.bounds.min.x
            //             && other.bounds.min.x < key.bounds.max.x)
            //             || (other.bounds.max.x <= key.bounds.max.x
            //                 && other.bounds.max.x > key.bounds.min.x))
            //     {
            //         stride.max_y = 2u32
            //             .pow((key.level as i32 - other.level as i32).abs() as u32)
            //             .max(stride.max_y)
            //     }
            // }
            // self.terrain_data.stitch_mesh(key, &stride);
            // self.injector.push(TerrainTask::StitchMesh(*key, stride));
        }
    }

    #[profiling::function]
    pub fn render<'a>(&'a self, regions: &[Region]) -> Vec<TerrainRenderBundle> {
        self.terrain_data.render(regions)
    }

    #[profiling::function]
    pub fn tree(&self) -> RwLockReadGuard<Tree> {
        self.terrain_data.tree.read()
    }

    #[profiling::function]
    pub fn mesh_cache(&self) -> RwLockReadGuard<Cache<ChunkCacheKey, ChunkMesh>> {
        self.terrain_data.mesh_cache.read()
    }

    pub fn set_isolevel(&self, isolevel: f32) {
        self.terrain_data.set_isolevel(isolevel);
        self.injector.push(TerrainTask::InvalidateTriangle);
    }
}

struct TerrainData {
    tree: RwLock<Tree>,
    isolevel: RwLock<f32>,
    chunk_cache: RwLock<Cache<ChunkCacheKey, Chunk>>,
    mesh_cache: RwLock<Cache<ChunkCacheKey, ChunkMesh>>,
    generate_voxel_pipeline: Option<ComputePipeline>,
    generate_triangle_pipeline: Option<ComputePipeline>,
    render_pipeline: Option<RenderPipeline>,
    render_bind_group_layout: Option<BindGroupLayout>,
    render_target_format: Option<TextureFormat>,
}

impl TerrainData {
    fn new() -> Self {
        Self {
            chunk_cache: RwLock::new(Cache::new(128)),
            mesh_cache: RwLock::new(Cache::new(256)),
            tree: RwLock::new(Tree::new()),
            isolevel: RwLock::new(0.5),
            generate_voxel_pipeline: None,
            generate_triangle_pipeline: None,
            render_pipeline: None,
            render_bind_group_layout: None,
            render_target_format: None,
        }
    }

    fn init(&mut self, instance: &Instance, target_format: TextureFormat) {
        self.init_generate_voxel_pipeline(instance);
        self.init_generate_triangle_pipeline(instance);
        self.init_render_pipeline(instance, target_format);
    }

    fn init_generate_voxel_pipeline(&mut self, instance: &Instance) {
        let device = instance.device();
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("terrain_voxel_bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("terrain_voxel_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader_module =
            device.create_shader_module(&include_wgsl!("shaders/generate_voxel.wgsl"));
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("terrain_voxel_compute_pipeline"),
            entry_point: "main",
            module: &shader_module,
            layout: Some(&pipeline_layout),
        });

        self.generate_voxel_pipeline = Some(pipeline);
    }

    fn init_generate_triangle_pipeline(&mut self, instance: &Instance) {
        let device = instance.device();
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("terrain_triangle_bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("terrain_triangle_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader_module =
            device.create_shader_module(&include_wgsl!("shaders/generate_triangle.wgsl"));
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("terrain_triangle_compute_pipeline"),
            entry_point: "main",
            module: &shader_module,
            layout: Some(&pipeline_layout),
        });

        self.generate_triangle_pipeline = Some(pipeline);
    }

    pub fn init_render_pipeline(&mut self, instance: &Instance, target_format: TextureFormat) {
        let device = instance.device();
        self.render_bind_group_layout =
            Some(device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("terrain_render_bind_group_layout"),
                entries: &[
                    // world matrix
                    BindGroupLayoutEntry {
                        binding: 0,
                        visibility: ShaderStages::VERTEX,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // view + projection matrix
                    BindGroupLayoutEntry {
                        binding: 1,
                        visibility: ShaderStages::VERTEX,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            }));
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("terrain_render_pipeline_layout"),
            bind_group_layouts: &[self.render_bind_group_layout.as_ref().unwrap()],
            push_constant_ranges: &[],
        });
        let shader_module = device.create_shader_module(&include_wgsl!("shaders/render.wgsl"));
        self.render_pipeline = Some(device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("terrain_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader_module,
                entry_point: "main",
                buffers: &[VertexBufferLayout {
                    array_stride: size_of::<VertexData>() as u64,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &vertex_attr_array![
                        0 => Float32x4,
                        1 => Float32x4,
                    ],
                }],
            },
            primitive: PrimitiveState {
                // polygon_mode: PolygonMode::Line,
                cull_mode: Some(Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: CompareFunction::Less,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader_module,
                entry_point: "main",
                targets: &[ColorTargetState {
                    format: target_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                }],
            }),
        }));
        self.render_target_format = Some(target_format);
    }

    #[profiling::function]
    fn generate_chunk(&self, instance: &Instance, key: &ChunkCacheKey) -> Option<TerrainTask> {
        let device = instance.device();
        {
            let mesh_cache = self.mesh_cache.read();
            if let Some(mesh) = mesh_cache.get(key) {
                if mesh.render_bundle().is_none() {
                    return Some(TerrainTask::GenerateMeshResouces(*key));
                } else {
                    return None;
                }
            }
        }
        {
            let chunk_cache = self.chunk_cache.read();
            let chunk = chunk_cache.get(key);
            if let Some(chunk) = chunk {
                if chunk.triangle_buffer().is_none() {
                    return Some(TerrainTask::RegenerateTriangle(*key));
                }
                return Some(TerrainTask::GenerateMesh(*key));
            }
        }
        let mut chunk = Chunk::new(key.bounds, key.level, size3(32, 32, 1 << (key.level - 2)));
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        chunk.generate_voxel(
            instance,
            &mut encoder,
            self.generate_voxel_pipeline.as_ref().unwrap(),
            true,
        );

        chunk.generate_triangle(
            instance,
            &mut encoder,
            self.generate_triangle_pipeline.as_ref().unwrap(),
            true,
            *self.isolevel.read(),
        );
        instance.queue().submit(std::iter::once(encoder.finish()));
        Some(TerrainTask::WriteChunk(*key, chunk))
    }

    #[profiling::function]
    fn write_chunk(&self, key: &ChunkCacheKey, chunk: Chunk) -> Option<TerrainTask> {
        loop {
            let chunk_cache = self.chunk_cache.try_write();
            if chunk_cache.is_none() {
                continue;
            }
            chunk_cache.unwrap().insert(key, chunk);
            break;
        }
        Some(TerrainTask::GenerateMesh(*key))
    }

    #[profiling::function]
    fn generate_mesh(&self, key: &ChunkCacheKey) -> Option<TerrainTask> {
        {
            let mesh_cache = self.mesh_cache.read();
            if let Some(mesh) = mesh_cache.get(key) {
                if mesh.render_bundle().is_none() {
                    return Some(TerrainTask::GenerateMeshResouces(*key));
                } else {
                    return None;
                }
            }
        }
        let chunk_cache = self.chunk_cache.try_write();
        if chunk_cache.is_none() {
            return Some(TerrainTask::GenerateMesh(*key));
        }
        let mut chunk_cache = chunk_cache.unwrap();
        let chunk = chunk_cache.get_mut(key);
        if chunk.is_none() || chunk.as_ref().unwrap().triangle_buffer().is_none() {
            return Some(TerrainTask::GenerateChunk(*key));
        };
        let chunk = chunk.unwrap();

        chunk.map_triangle_buffer();
        let triangles = chunk.get_mapped_triangle_buffer();
        let mut mesh = Mesh::from_triangles(triangles);
        mesh.calculate_normals();
        chunk.unmap_triangle_buffer();

        chunk.map_voxel_buffer();
        let edge_voxel =
            EdgeVoxel::from_voxels(&chunk.get_mapped_voxel_buffer(), chunk.voxel_count());
        chunk.unmap_voxel_buffer();

        let mesh = ChunkMesh::new(key.bounds, mesh, chunk.voxel_count(), edge_voxel);
        Some(TerrainTask::WriteMesh(*key, mesh))
    }

    #[profiling::function]
    fn write_mesh(&self, key: &ChunkCacheKey, mesh: ChunkMesh) -> Option<TerrainTask> {
        loop {
            let mesh_cache = self.mesh_cache.try_write();
            if mesh_cache.is_none() {
                continue;
            }
            mesh_cache.unwrap().insert(key, mesh);
            break;
        }
        Some(TerrainTask::GenerateMeshResouces(*key))
    }

    #[profiling::function]
    fn generate_mesh_resources(
        &self,
        instance: &Instance,
        camera_uniform_buffer: &Buffer,
        key: &ChunkCacheKey,
    ) -> Option<TerrainTask> {
        let render_pipeline = self.render_pipeline.as_ref().unwrap();
        let render_bind_group_layout = self.render_bind_group_layout.as_ref().unwrap();
        let mesh_cache = self.mesh_cache.try_write();
        if mesh_cache.is_none() {
            return Some(TerrainTask::GenerateMeshResouces(*key));
        }
        let mut mesh_cache = mesh_cache.unwrap();
        if let Some(mesh) = mesh_cache.get_mut(key) {
            mesh.create_render_resources(
                instance,
                render_pipeline,
                render_bind_group_layout,
                camera_uniform_buffer,
                self.render_target_format.unwrap(),
            );
            None
        } else {
            Some(TerrainTask::GenerateMesh(*key))
        }
    }

    #[profiling::function]
    fn update_last_accessed(&self, keys: &[ChunkCacheKey]) {
        let mut mesh_cache = self.mesh_cache.write();
        for key in keys {
            mesh_cache.update_last_accessed(key);
        }
    }

    #[profiling::function]
    fn render<'a>(&'a self, regions: &[Region]) -> Vec<TerrainRenderBundle> {
        let mut bundles = vec![];
        let mesh_cache = self.mesh_cache.read();
        let tree = self.tree.read();
        let mut stack = vec![];
        for node in tree.root_nodes() {
            if regions.iter().any(|x| node.intersects_region(x)) {
                stack.push(node);
            }
        }
        while let Some(node) = stack.pop() {
            if node.sub_nodes().is_none() {
                let bounds = node.bounds();
                let level = node.level();
                let key = ChunkCacheKey { bounds, level };
                if let Some(mesh) = mesh_cache.get(&key) {
                    if mesh.render_bundle().is_some() {
                        bundles.push(TerrainRenderBundle {
                            key,
                            guard: self.mesh_cache.read(),
                        })
                    }
                }
            } else {
                let mut sub_nodes_intersect = vec![];
                for sub_node in node.sub_nodes().unwrap() {
                    if regions.iter().any(|x| sub_node.intersects_region(x)) {
                        sub_nodes_intersect.push(sub_node);
                    }
                }
                // If not all sub node is renderable, render the parent
                if sub_nodes_intersect.iter().all(|x| x.sub_nodes().is_none()) {
                    if sub_nodes_intersect.iter().any(|x| {
                        let bounds = x.bounds();
                        let level = x.level();
                        let key = ChunkCacheKey { bounds, level };
                        if let Some(mesh) = mesh_cache.get(&key) {
                            mesh.render_bundle().is_none()
                        } else {
                            true
                        }
                    }) {
                        let bounds = node.bounds();
                        let level = node.level();
                        let key = ChunkCacheKey { bounds, level };
                        if let Some(mesh) = mesh_cache.get(&key) {
                            if mesh.render_bundle().is_some() {
                                bundles.push(TerrainRenderBundle {
                                    key,
                                    guard: self.mesh_cache.read(),
                                })
                            }
                        }
                    } else {
                        stack.append(&mut sub_nodes_intersect);
                    }
                } else {
                    stack.append(&mut sub_nodes_intersect);
                }
            }
        }
        bundles
    }

    #[profiling::function]
    fn set_isolevel(&self, isolevel: f32) {
        *self.isolevel.write() = isolevel;
    }

    #[profiling::function]
    fn regenerate_triangle(&self, instance: &Instance, key: &ChunkCacheKey) -> Option<TerrainTask> {
        loop {
            let chunk_cache = self.chunk_cache.try_write();
            if chunk_cache.is_none() {
                continue;
            }
            if let Some(chunk) = chunk_cache.unwrap().get_mut(key) {
                let device = instance.device();
                let mut encoder =
                    device.create_command_encoder(&CommandEncoderDescriptor { label: None });
                chunk.generate_triangle(
                    instance,
                    &mut encoder,
                    self.generate_triangle_pipeline.as_ref().unwrap(),
                    true,
                    *self.isolevel.read(),
                );
                instance.queue().submit(std::iter::once(encoder.finish()));
                return Some(TerrainTask::GenerateMesh(*key));
            }
            break;
        }
        None
    }

    #[profiling::function]
    fn invalidate_triangle(&self) -> Option<TerrainTask> {
        loop {
            let chunk_cache = self.chunk_cache.try_write();
            if chunk_cache.is_none() {
                continue;
            }
            for chunk in chunk_cache.unwrap().values_mut() {
                chunk.clear_triangle_buffer();
            }
            loop {
                let mesh_cache = self.mesh_cache.try_write();
                if mesh_cache.is_none() {
                    continue;
                }
                mesh_cache.unwrap().clear();
                break;
            }
            break;
        }
        None
    }

    #[profiling::function]
    fn stitch_mesh(&self, key: &ChunkCacheKey, stride: &StitchStride) -> Option<TerrainTask> {
        let mesh_cache = self.mesh_cache.read();
        if let Some(mesh) = mesh_cache.get(key) {
            if mesh.render_bundle().is_some() {
                mesh.stitch_edges(stride.min_x, stride.max_x, stride.min_y, stride.max_y);
            }
        }
        None
    }
}

impl Drop for Terrain {
    fn drop(&mut self) {
        *self.guard.lock().unwrap() = false;
        self.condvar.notify_all();
    }
}

pub struct TerrainRenderBundle<'a> {
    key: ChunkCacheKey,
    guard: RwLockReadGuard<'a, Cache<ChunkCacheKey, ChunkMesh>>,
}

impl<'a, 'b> From<&'b TerrainRenderBundle<'a>> for &'b RenderBundle
where
    'a: 'b,
{
    fn from(item: &'b TerrainRenderBundle<'a>) -> &'b RenderBundle {
        item.guard.get(&item.key).unwrap().render_bundle().unwrap()
    }
}
