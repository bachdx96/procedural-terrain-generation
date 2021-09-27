mod game;
mod gfx;
mod windowing;

use game::Game;
use gfx::Instance;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windowing::Window;
use winit::{
    event::{Event, WindowEvent},
    event_loop::ControlFlow,
};

fn main() {
    env_logger::init();
    let window = Window::new();
    let instance = Arc::new(Instance::new(&window));
    let mut game = Game::new(instance.clone());
    game.init(window.winit_window());
    let mut prev_time = Instant::now();
    window.run(move |window, event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
        instance.device().poll(wgpu::Maintain::Poll);
        let now = Instant::now();
        game.handle_event(window, &event);
        let duration = now.duration_since(prev_time);
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
            } if window_id == window.id() => *control_flow = ControlFlow::Exit,
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                instance.recreate_swapchain(size);
            }
            Event::RedrawEventsCleared => {
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                if duration >= Duration::from_secs_f64(1.0 / 60.0) {
                    prev_time = now;
                    game.step(window, duration);
                    game.render(window);
                }
            }
            _ => {}
        }
    });
}
