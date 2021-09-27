use crate::gfx::Instance;
use imgui::{internal::RawWrapper, Context, FontConfig, FontSource, TextureId, Ui};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use std::{
    collections::HashMap,
    mem::{size_of, size_of_val},
    ptr::copy_nonoverlapping,
    time::Instant,
};
use wgpu::util::DeviceExt;
use wgpu::util::StagingBelt;
use wgpu::*;
use winit::{event::Event, window::Window};

#[derive(Copy, Clone, bytemuck::Zeroable, Debug, bytemuck::Pod, Default)]
#[repr(C)]
struct UniformData {
    scale: [f32; 2],
    translate: [f32; 2],
}

pub struct ImguiRenderer {
    context: Context,
    platform: WinitPlatform,
    pipeline: Option<RenderPipeline>,
    sampler: Option<Sampler>,
    texture_bind_group_layout: Option<BindGroupLayout>,
    uniform_bind_group_layout: Option<BindGroupLayout>,
    texture_bind_groups: HashMap<TextureId, BindGroup>,
    last_frame: Instant,
    vertex_buffer: Option<(Buffer, BufferSize)>,
    index_buffer: Option<(Buffer, BufferSize)>,
    uniform_buffer: Option<(Buffer, BindGroup)>,
    draw_data: Option<*const imgui::DrawData>,
}

impl ImguiRenderer {
    pub fn new() -> Self {
        let mut context = Context::create();
        context.io_mut().backend_flags |= imgui::BackendFlags::RENDERER_HAS_VTX_OFFSET;
        let platform = WinitPlatform::init(&mut context);
        Self {
            context,
            platform,
            pipeline: None,
            sampler: None,
            texture_bind_group_layout: None,
            uniform_bind_group_layout: None,
            texture_bind_groups: HashMap::new(),
            last_frame: Instant::now(),
            vertex_buffer: None,
            index_buffer: None,
            uniform_buffer: None,
            draw_data: None,
        }
    }

    pub fn init(&mut self, window: &Window, instance: &Instance) {
        self.platform
            .attach_window(self.context.io_mut(), window, HiDpiMode::Default);
        let hidpi_factor = self.platform.hidpi_factor();
        let font_size = (13.0 * hidpi_factor) as f32;
        self.context.fonts().clear_fonts();
        self.context
            .fonts()
            .add_font(&[FontSource::DefaultFontData {
                config: Some(FontConfig {
                    size_pixels: font_size,
                    ..FontConfig::default()
                }),
            }]);

        // Create pipeline objects
        self.create_texture_bind_group_layout(instance);
        self.create_sampler(instance);
        self.create_uniform_bind_group_layout(instance);
        self.create_pipeline(instance);
        self.create_font_texture(instance);
    }

    pub fn handle_event(&mut self, window: &Window, event: &Event<()>) {
        let io = self.context.io_mut();
        self.platform.handle_event(io, window, event);
    }

    #[profiling::function]
    pub fn draw<F>(&mut self, window: &Window, mut draw_fn: F)
    where
        F: FnMut(&mut Ui),
    {
        let draw_data = {
            let io = self.context.io_mut();
            self.platform.prepare_frame(io, window).unwrap();
            let now = Instant::now();
            io.update_delta_time(now.duration_since(self.last_frame));
            self.last_frame = now;
            let mut ui = self.context.frame();
            draw_fn(&mut ui);
            self.platform.prepare_render(&ui, window);
            ui.render()
        };
        self.draw_data = Some(draw_data)
    }

    #[profiling::function]
    pub fn update_buffer(
        &mut self,
        instance: &Instance,
        staging_belt: &mut StagingBelt,
        encoder: &mut CommandEncoder,
    ) {
        if self.draw_data.is_none() {
            return;
        }
        let draw_data = unsafe { &*self.draw_data.unwrap() };
        let device = instance.device();
        let fb_width = draw_data.display_size[0] * draw_data.framebuffer_scale[0];
        let fb_height = draw_data.display_size[1] * draw_data.framebuffer_scale[1];
        if !(fb_width > 0.0
            && fb_height > 0.0
            && draw_data.total_vtx_count > 0
            && draw_data.total_idx_count > 0)
        {
            return;
        }
        let mut vertex_buffer_size =
            draw_data.total_vtx_count as u64 * size_of::<imgui::DrawVert>() as u64;
        let mut index_buffer_size =
            draw_data.total_idx_count as u64 * size_of::<imgui::DrawIdx>() as u64;
        vertex_buffer_size += vertex_buffer_size % COPY_BUFFER_ALIGNMENT;
        index_buffer_size += index_buffer_size % COPY_BUFFER_ALIGNMENT;
        let vertex_buffer_size = BufferSize::new(vertex_buffer_size).unwrap();
        let index_buffer_size = BufferSize::new(index_buffer_size).unwrap();

        if self.vertex_buffer.is_none()
            || self.vertex_buffer.as_ref().unwrap().1 < vertex_buffer_size
        {
            self.vertex_buffer = Some((
                device.create_buffer(&BufferDescriptor {
                    label: Some("imgui_renderer_vertex_buffer"),
                    size: vertex_buffer_size.into(),
                    usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                vertex_buffer_size,
            ))
        }
        if self.index_buffer.is_none()
            || self.index_buffer.as_ref().unwrap().1 < index_buffer_size as _
        {
            self.index_buffer = Some((
                device.create_buffer(&BufferDescriptor {
                    label: Some("imgui_renderer_index_buffer"),
                    size: index_buffer_size.into(),
                    usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                index_buffer_size,
            ))
        }

        if self.uniform_buffer.is_none() {
            let uniform = device.create_buffer(&BufferDescriptor {
                label: Some("imgui_renderer_uniform_buffer"),
                size: size_of::<UniformData>() as _,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &uniform,
                        offset: 0,
                        size: None,
                    }),
                }],
                label: Some("imgui_font_bind_group"),
                layout: self.uniform_bind_group_layout.as_ref().unwrap(),
            });
            self.uniform_buffer = Some((uniform, bind_group));
        }

        let scale = [
            2. / draw_data.display_size[0],
            2. / draw_data.display_size[1],
        ];
        let translate = [
            -1. - draw_data.display_pos[0] * scale[0],
            -1. - draw_data.display_pos[1] * scale[1],
        ];
        staging_belt
            .write_buffer(
                encoder,
                &self.uniform_buffer.as_ref().unwrap().0,
                0,
                BufferSize::new(size_of::<UniformData>() as u64).unwrap(),
                device,
            )
            .copy_from_slice(bytemuck::bytes_of(&UniformData { scale, translate }));

        {
            let mut global_vtx_offset = 0;
            let mut buffer_view = staging_belt.write_buffer(
                encoder,
                &self.vertex_buffer.as_ref().unwrap().0,
                0,
                self.vertex_buffer.as_ref().unwrap().1,
                device,
            );
            for draw_list in draw_data.draw_lists() {
                let draw_vtx_buffer_size = size_of_val(draw_list.vtx_buffer());
                unsafe {
                    copy_nonoverlapping(
                        draw_list.vtx_buffer().as_ptr() as *const u8,
                        buffer_view[global_vtx_offset as usize * size_of::<imgui::DrawVert>()..]
                            .as_mut_ptr(),
                        draw_vtx_buffer_size,
                    );
                    global_vtx_offset += draw_list.vtx_buffer().len();
                }
            }
        }
        {
            let mut global_idx_offset = 0;
            let mut index_buffer_view = staging_belt.write_buffer(
                encoder,
                &self.index_buffer.as_ref().unwrap().0,
                0,
                self.index_buffer.as_ref().unwrap().1,
                device,
            );

            for draw_list in draw_data.draw_lists() {
                let draw_idx_buffer_size = size_of_val(draw_list.idx_buffer());
                unsafe {
                    copy_nonoverlapping(
                        draw_list.idx_buffer().as_ptr() as *const u8,
                        index_buffer_view
                            [global_idx_offset as usize * size_of::<imgui::DrawIdx>()..]
                            .as_mut_ptr(),
                        draw_idx_buffer_size,
                    );
                }
                global_idx_offset += draw_list.idx_buffer().len();
            }
        }
    }

    #[profiling::function]
    pub fn render<'a>(&'a mut self, render_pass: &mut RenderPass<'a>) {
        if self.draw_data.is_none() {
            return;
        }
        let draw_data = unsafe { &*self.draw_data.unwrap() };
        let fb_width = draw_data.display_size[0] * draw_data.framebuffer_scale[0];
        let fb_height = draw_data.display_size[1] * draw_data.framebuffer_scale[1];
        if !(fb_width > 0.0
            && fb_height > 0.0
            && draw_data.total_vtx_count > 0
            && draw_data.total_idx_count > 0)
        {
            return;
        }

        let mut global_vtx_offset = 0;
        let mut global_idx_offset = 0;

        {
            reset_render_state(
                &self.vertex_buffer.as_ref().unwrap().0,
                &self.index_buffer.as_ref().unwrap().0,
                &self.uniform_buffer.as_ref().unwrap().1,
                self.pipeline.as_ref().unwrap(),
                render_pass,
                draw_data,
            );
            for draw_list in draw_data.draw_lists() {
                let clip_off = draw_data.display_pos;
                let clip_scale = draw_data.framebuffer_scale;
                for draw_command in draw_list.commands() {
                    match draw_command {
                        imgui::DrawCmd::ResetRenderState => {
                            reset_render_state(
                                &self.vertex_buffer.as_ref().unwrap().0,
                                &self.index_buffer.as_ref().unwrap().0,
                                &self.uniform_buffer.as_ref().unwrap().1,
                                self.pipeline.as_ref().unwrap(),
                                render_pass,
                                draw_data,
                            );
                        }
                        imgui::DrawCmd::RawCallback {
                            callback: cb,
                            raw_cmd: cmd,
                        } => unsafe { cb(draw_list.raw(), cmd) },
                        imgui::DrawCmd::Elements {
                            count,
                            cmd_params:
                                imgui::DrawCmdParams {
                                    clip_rect,
                                    texture_id,
                                    vtx_offset,
                                    idx_offset,
                                },
                        } => {
                            let clip_rect = [
                                (clip_rect[0] - clip_off[0]) * clip_scale[0],
                                (clip_rect[1] - clip_off[1]) * clip_scale[1],
                                (clip_rect[2] - clip_off[0]) * clip_scale[0],
                                (clip_rect[3] - clip_off[1]) * clip_scale[1],
                            ];
                            if clip_rect[2] < clip_rect[0] || clip_rect[3] < clip_rect[1] {
                                continue;
                            }
                            let scissor_rect = [
                                (clip_rect[0] as u32).clamp(0, fb_width as u32),
                                (clip_rect[1] as u32).clamp(0, fb_height as u32),
                                ((clip_rect[2] - clip_rect[0]) as u32)
                                    .clamp(0, (fb_width - clip_rect[0]) as u32),
                                ((clip_rect[3] - clip_rect[1]) as u32)
                                    .clamp(0, (fb_height - clip_rect[1]) as u32),
                            ];
                            if scissor_rect[2] == 0 || scissor_rect[3] == 0 {
                                continue;
                            }
                            render_pass.set_scissor_rect(
                                scissor_rect[0],
                                scissor_rect[1],
                                scissor_rect[2],
                                scissor_rect[3],
                            );
                            render_pass.set_bind_group(
                                0,
                                self.texture_bind_groups.get(&texture_id).as_ref().unwrap(),
                                &[],
                            );
                            render_pass.draw_indexed(
                                (idx_offset + global_idx_offset) as _
                                    ..(idx_offset + global_idx_offset + count) as _,
                                (vtx_offset + global_vtx_offset) as _,
                                0..1,
                            );
                        }
                    };
                }
                global_vtx_offset += draw_list.vtx_buffer().len();
                global_idx_offset += draw_list.idx_buffer().len();
            }
        }
    }

    fn create_sampler(&mut self, instance: &Instance) {
        let device = instance.device();
        let sampler = device.create_sampler(&SamplerDescriptor {
            ..Default::default()
        });
        self.sampler = Some(sampler);
    }

    fn create_texture_bind_group_layout(&mut self, instance: &Instance) {
        let device = instance.device();
        let texture_bind_group_layout =
            device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("imgui_bind_group_layout"),
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Sampler {
                            comparison: false,
                            filtering: false,
                        },
                        count: None,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Texture {
                            view_dimension: TextureViewDimension::D2,
                            sample_type: TextureSampleType::Float { filterable: false },
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        self.texture_bind_group_layout = Some(texture_bind_group_layout);
    }

    fn create_uniform_bind_group_layout(&mut self, instance: &Instance) {
        let device = instance.device();
        // TODO: Use push constants instead of uniform buffer
        // Note: push constants only available in native not webgpu
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("imgui_uniform_bind_group_layout"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        self.uniform_bind_group_layout = Some(uniform_bind_group_layout);
    }

    fn create_pipeline(&mut self, instance: &Instance) {
        let device = instance.device();
        let shader_module = device.create_shader_module(&include_wgsl!("shaders/render.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                self.texture_bind_group_layout.as_ref().unwrap(),
                self.uniform_bind_group_layout.as_ref().unwrap(),
            ],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader_module,
                entry_point: "vs_main",
                buffers: &[VertexBufferLayout {
                    attributes: &vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Unorm8x4,
                    ],
                    step_mode: VertexStepMode::Vertex,
                    array_stride: size_of::<imgui::DrawVert>() as _,
                }],
            },
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader_module,
                entry_point: "fs_main",
                targets: &[ColorTargetState {
                    format: TextureFormat::Bgra8UnormSrgb,
                    write_mask: ColorWrites::ALL,
                    blend: Some(BlendState {
                        alpha: BlendComponent {
                            src_factor: BlendFactor::OneMinusSrcAlpha,
                            dst_factor: BlendFactor::Zero,
                            operation: BlendOperation::Add,
                        },
                        color: BlendComponent {
                            src_factor: BlendFactor::SrcAlpha,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                    }),
                }],
            }),
        });
        self.pipeline = Some(pipeline);
    }

    fn create_font_texture(&mut self, instance: &Instance) {
        let device = instance.device();
        let queue = instance.queue();
        let font_texture = {
            let mut fonts = self.context.fonts();
            let font_data = fonts.build_rgba32_texture();
            device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("imgui_font_texture"),
                    size: Extent3d {
                        width: font_data.width,
                        height: font_data.height,
                        depth_or_array_layers: 1,
                    },
                    dimension: TextureDimension::D2,
                    sample_count: 1,
                    mip_level_count: 1,
                    format: TextureFormat::Rgba8Unorm,
                    usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
                },
                font_data.data,
            )
        };
        queue.submit(None);

        let font_texture_view = font_texture.create_view(&TextureViewDescriptor {
            ..Default::default()
        });
        self.register_texture(instance, &font_texture_view, TextureId::from(0));
    }

    pub fn register_texture(
        &mut self,
        instance: &Instance,
        texture_view: &TextureView,
        texture_id: TextureId,
    ) {
        let device = instance.device();
        let font_bind_group = device.create_bind_group(&BindGroupDescriptor {
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Sampler(self.sampler.as_ref().unwrap()),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(texture_view),
                },
            ],
            label: Some("imgui_font_bind_group"),
            layout: self.texture_bind_group_layout.as_ref().unwrap(),
        });
        self.texture_bind_groups.insert(texture_id, font_bind_group);
    }
}

fn reset_render_state<'a>(
    vertex_buffer: &'a Buffer,
    index_buffer: &'a Buffer,
    uniform_bind_group: &'a BindGroup,
    pipeline: &'a RenderPipeline,
    render_pass: &mut RenderPass<'a>,
    draw_data: &imgui::DrawData,
) {
    let width = draw_data.display_size[0] * draw_data.framebuffer_scale[0];
    let height = draw_data.display_size[1] * draw_data.framebuffer_scale[1];
    render_pass.set_pipeline(pipeline);
    render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
    render_pass.set_index_buffer(
        index_buffer.slice(..),
        match size_of::<imgui::DrawIdx>() {
            2 => IndexFormat::Uint16,
            4 => IndexFormat::Uint32,
            _ => unimplemented!(),
        },
    );
    render_pass.set_viewport(0., 0., width, height, 0., 1.);
    render_pass.set_bind_group(1, uniform_bind_group, &[]);
}
