mod base;
mod camera;
mod mesh;
mod object;
mod terrain;
mod ui;

use crate::gfx::Instance;
use base::Region;
use camera::Camera;
use euclid::{point3, vec3, Rotation2D, Scale};
use futures::task::SpawnExt;
use std::sync::Arc;
use std::time::Duration;
use terrain::{Terrain, TerrainRegion};
use ui::{ImguiRenderer, TerrainVisualizer};
use wgpu::util::StagingBelt;
use wgpu::*;
use winit::{event::Event, window::Window};

pub struct Game {
    instance: Arc<Instance>,
    imgui_renderer: ImguiRenderer,
    terrain_visualizer: TerrainVisualizer,
    camera: Camera,
    terrain: Terrain,
    render_target_view: Option<TextureView>,
    depth_stencil_view: Option<TextureView>,
    staging_belt: StagingBelt,
    regions: Vec<Region>,
    isolevel: f32,
}

impl Game {
    pub fn new(instance: Arc<Instance>) -> Self {
        let camera = Camera::new(
            point3(0.0, 0.0, 0.3),
            vec3(1.0, 0.0, 0.0),
            std::f32::consts::PI / 4.0,
            640.0 / 480.0,
            0.001,
            9000.0,
        );
        let regions = camera.lod_regions(1.0, 2.0, 3);
        Self {
            instance,
            imgui_renderer: ImguiRenderer::new(),
            camera,
            terrain: Terrain::new(),
            terrain_visualizer: TerrainVisualizer::new(Scale::new(32.0)),
            render_target_view: None,
            depth_stencil_view: None,
            staging_belt: StagingBelt::new(0x100),
            regions,
            isolevel: 0.5,
        }
    }

    #[profiling::function]
    pub fn render(&mut self, _window: &Window) {
        let target = self.instance.surface().get_current_frame().unwrap();
        let view = target
            .output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .instance
            .device()
            .create_command_encoder(&CommandEncoderDescriptor { label: None });
        self.imgui_renderer
            .update_buffer(&self.instance, &mut self.staging_belt, &mut encoder);
        self.camera
            .update_buffer(&self.instance, &mut self.staging_belt, &mut encoder);
        {
            let mut rp = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLUE),
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });
            self.imgui_renderer.render(&mut rp);
        }
        {
            let x = self.terrain.render(&self.regions);
            let mut rp = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[RenderPassColorAttachment {
                    view: self.render_target_view.as_ref().unwrap(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: true,
                    },
                }],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.depth_stencil_view.as_ref().unwrap(),
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(1.0),
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });
            rp.execute_bundles(x.iter().map(|x| x.into()));
        }
        self.staging_belt.finish();
        let command_buffer = encoder.finish();
        self.instance
            .queue()
            .submit(std::iter::once(command_buffer));
        self.instance
            .async_pool()
            .spawn(self.staging_belt.recall())
            .unwrap();
    }

    #[profiling::function]
    pub fn step(&mut self, window: &Window, elapsed_time: Duration) {
        let mut moved = false;
        let terrain_visualizer = &self.terrain_visualizer;
        let camera = &mut self.camera;
        let terrain = &self.terrain;
        let regions = &mut self.regions;
        let mut isolevel_changed = false;
        let mut isolevel = &mut self.isolevel;
        self.imgui_renderer.draw(window, |ui| {
            let mut direction = camera.direction().xy();
            let mut speed = 0.0;
            if ui.is_key_down(imgui::Key::UpArrow) {
                speed += 1.0 * elapsed_time.as_secs_f32();
                moved = true;
            }
            if ui.is_key_down(imgui::Key::DownArrow) {
                speed -= 1.0 * elapsed_time.as_secs_f32();
                moved = true;
            }
            if ui.is_key_down(imgui::Key::LeftArrow) {
                direction = Rotation2D::radians(2.0 * elapsed_time.as_secs_f32())
                    .transform_vector(direction);
                moved = true;
            }
            if ui.is_key_down(imgui::Key::RightArrow) {
                direction = Rotation2D::radians(-2.0 * elapsed_time.as_secs_f32())
                    .transform_vector(direction);
                moved = true;
            }
            if moved {
                camera.move_by(&(direction * speed).extend(0.0));
                camera.look_in_direction(&direction.extend(-0.1));
                std::mem::swap(regions, &mut camera.lod_regions(1.0, 2.0, 3));
            }
            imgui::Window::new(imgui::im_str!("Terrain Chunk Viewer"))
                .size([640.0, 480.0], imgui::Condition::Once)
                .build(ui, || {
                    terrain_visualizer.draw(ui, terrain, camera, regions);
                });
            imgui::Window::new(imgui::im_str!("Scene Viewer"))
                .size([640.0, 480.0], imgui::Condition::Once)
                .always_auto_resize(true)
                .build(ui, || {
                    imgui::Slider::new(imgui::im_str!("isolevel"))
                        .range(0.0..=1.0)
                        .build(ui, &mut isolevel);
                    isolevel_changed = ui.is_item_deactivated();
                    imgui::Image::new(1.into(), [640.0, 480.0])
                        .border_col([1.0, 0.0, 0.0, 1.0])
                        .build(ui)
                });
            // ui.show_demo_window(&mut true);
        });
        if isolevel_changed {
            terrain.set_isolevel(self.isolevel);
        }
        terrain.update_terrain(
            self.camera.position(),
            regions
                .iter()
                .rev()
                .enumerate()
                .map(|(i, region)| TerrainRegion {
                    region: region.clone(),
                    level: ((9 - regions.len() as u32)..=8).nth(i).unwrap(),
                })
                .collect::<Vec<_>>()
                .as_slice(),
        );
        profiling::finish_frame!();
    }

    pub fn init(&mut self, window: &Window) {
        self.imgui_renderer.init(window, &self.instance);
        self.camera.init(&self.instance);
        self.init_render_target();
        self.terrain.init(
            self.instance.clone(),
            TextureFormat::Rgba8Unorm,
            self.camera.buffer(),
            0.5,
        );
    }

    fn init_render_target(&mut self) {
        let device = &self.instance.device();
        let render_target = device.create_texture(&TextureDescriptor {
            label: Some("scene_render_target"),
            size: Extent3d {
                width: 640,
                height: 480,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        });
        self.render_target_view =
            Some(render_target.create_view(&TextureViewDescriptor::default()));
        self.imgui_renderer.register_texture(
            &self.instance,
            self.render_target_view.as_ref().unwrap(),
            1.into(),
        );
        let depth_stencil = device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: 640,
                height: 480,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            label: Some("scene_depth_stencil"),
        });

        self.depth_stencil_view =
            Some(depth_stencil.create_view(&TextureViewDescriptor::default()));
    }

    #[profiling::function]
    pub fn handle_event(&mut self, window: &Window, event: &Event<()>) {
        self.imgui_renderer.handle_event(window, event);
    }
}
