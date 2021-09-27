use crate::game::base::Region;
use crate::game::base::WorldSpace;
use crate::game::camera::Camera;
use crate::game::terrain::{ChunkCacheKey, Terrain};
use euclid::{point2, vec2, Box2D, Point2D, Scale, Transform2D};
use imgui::Ui;
use std::borrow::Borrow;

pub struct TerrainVisualizerSpace;

pub struct TerrainVisualizer {
    scale: Scale<f32, WorldSpace, TerrainVisualizerSpace>,
}

impl TerrainVisualizer {
    pub fn new(scale: Scale<f32, WorldSpace, TerrainVisualizerSpace>) -> Self {
        Self { scale }
    }

    #[profiling::function]
    pub fn draw(&self, ui: &Ui, terrain: &Terrain, camera: &Camera, regions: &[Region]) {
        // let scale_inversed = self.scale.inverse();
        let win_bounds = Box2D::<_, TerrainVisualizerSpace>::from_origin_and_size(
            ui.cursor_screen_pos().into(),
            ui.content_region_avail().into(),
        );
        let center = win_bounds.center();
        // let position = camera.position();
        let draw_list = ui.get_window_draw_list();
        // let view_width = win_bounds.width() * scale_inversed.get();
        // let view_height = win_bounds.height() * scale_inversed.get();
        // let view_bounds = Box2D::<_, WorldSpace>::new(
        //     point2(
        //         position.x - view_width / 2.0,
        //         position.y - view_height / 2.0,
        //     ),
        //     point2(
        //         position.x + view_width / 2.0,
        //         position.y + view_height / 2.0,
        //     ),
        // );
        // Draw terrain
        {
            let transform = (-camera.position().xy().to_vector())
                .to_transform()
                .then_scale(self.scale.get(), -self.scale.get())
                .then(&center.to_vector().to_transform().with_source());
            let tree = terrain.tree();
            let mesh_cache = terrain.mesh_cache();
            for (leaf, in_region) in tree
                .leaf_outside_regions_iter(regions)
                .zip(std::iter::repeat(false))
                .chain(
                    tree.leaf_intersect_regions_iter(regions)
                        .zip(std::iter::repeat(true)),
                )
            {
                let p0 = transform.transform_point(leaf.bounds().min.xy().to_f32());
                let p1 = transform.transform_point(leaf.bounds().max.xy().to_f32());
                let (border_color, fill_color) = if in_region {
                    let bounds = leaf.bounds();
                    let level = leaf.level();
                    let key = ChunkCacheKey { bounds, level };
                    let fill_color = if let Some(mesh) = mesh_cache.get(&key) {
                        if mesh.render_bundle().is_none() {
                            [0.0, 0.0, 1.0]
                        } else {
                            [0.0, 0.5, 1.0]
                        }
                    } else {
                        [1.0, 0.0, 0.0]
                    };
                    ([0.0, 1.0, 0.0], fill_color)
                } else {
                    ([0.0, 0.0, 1.0], [0.0, 0.0, 0.0])
                };
                if win_bounds.contains(p0) || win_bounds.contains(p1) {
                    if in_region {
                        draw_list
                            .add_rect(p0.into(), p1.into(), fill_color)
                            .filled(true)
                            .build();
                    }

                    draw_list
                        .add_rect(p0.into(), p1.into(), border_color)
                        .build();
                }
            }
        }
        // Draw regions
        {
            let transform = (-camera.position().xy().to_vector())
                .to_transform()
                .then_scale(self.scale.get(), -self.scale.get())
                .then(&center.to_vector().to_transform().with_source());
            for region in regions {
                let points = region.borrow().points().as_slice();
                for i in 0..points.len() {
                    let p0 = transform.transform_point(points[i]);
                    let p1 = transform.transform_point(points[(i + 1) % points.len()]);
                    draw_list
                        .add_line(p0.into(), p1.into(), [1.0, 0.0, 0.0])
                        .build();
                }
            }
        }
        // Draw camera shape
        {
            let camera_shape: [Point2D<f32, TerrainVisualizerSpace>; 4] = [
                point2(-0.5, 0.0),
                point2(-1.0, 1.0),
                point2(1.5, 0.0),
                point2(-1.0, -1.0),
            ];
            let angle = camera.direction().xy().angle_to(vec2(1.0, 0.0));
            let transform = Transform2D::rotation(angle)
                .then_scale(10.0, 10.0)
                .then_translate(center.to_vector());
            let p0 = transform.transform_point(camera_shape[0]);
            let p1 = transform.transform_point(camera_shape[1]);
            let p2 = transform.transform_point(camera_shape[2]);
            let p3 = transform.transform_point(camera_shape[3]);
            draw_list
                .add_triangle(p0.into(), p1.into(), p2.into(), [1.0, 0.0, 0.0])
                .filled(true)
                .build();
            draw_list
                .add_triangle(p0.into(), p3.into(), p2.into(), [1.0, 0.0, 0.0])
                .filled(true)
                .build();
        }
    }
}
