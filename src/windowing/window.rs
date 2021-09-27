use winit::event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget};

pub struct Window {
    winit_window: winit::window::Window,
    event_loop: EventLoop<()>,
}

impl Window {
    pub fn new() -> Self {
        let event_loop = EventLoop::new();
        let winit_window = winit::window::WindowBuilder::new()
            .with_maximized(true)
            .build(&event_loop)
            .unwrap();
        Self {
            winit_window,
            event_loop,
        }
    }

    /// Planned to write a event system but it seems too difficult
    /// to implement in Rust. For now, just make a simple wrapper
    /// around `winit::window::Window` object
    pub fn run<F>(self, mut f: F)
    where
        F: 'static
            + FnMut(
                &mut winit::window::Window,
                winit::event::Event<'_, ()>,
                &EventLoopWindowTarget<()>,
                &mut ControlFlow,
            ),
    {
        let event_loop = self.event_loop;
        let mut window = self.winit_window;
        event_loop.run(move |event, target, control_flow| {
            f(&mut window, event, target, control_flow);
        });
    }

    pub fn winit_window(&self) -> &winit::window::Window {
        &self.winit_window
    }
}
