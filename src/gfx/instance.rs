use crate::windowing::Window;
use futures::executor::block_on;
use futures::executor::ThreadPool;
use wgpu::*;

pub struct Instance {
    surface: Surface,
    device: Device,
    queue: Queue,
    adapter: wgpu::Adapter,
    async_pool: ThreadPool,
}

impl Instance {
    pub fn new(window: &Window) -> Self {
        let wgpu_instance = wgpu::Instance::new(Backends::all());
        let surface = unsafe { wgpu_instance.create_surface(window.winit_window()) };
        let adapter = block_on(wgpu_instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
        }))
        .unwrap();
        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::POLYGON_MODE_LINE,
                limits: adapter.limits(),
            },
            None,
        ))
        .unwrap();

        let size = window.winit_window().inner_size();

        let swapchain_format = surface.get_preferred_format(&adapter).unwrap();
        let sc_desc = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: swapchain_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Mailbox,
        };
        surface.configure(&device, &sc_desc);

        Self {
            surface,
            device,
            queue,
            adapter,
            async_pool: ThreadPool::new().unwrap(),
        }
    }

    pub fn recreate_swapchain(&self, size: winit::dpi::PhysicalSize<u32>) {
        let swapchain_format = self.surface.get_preferred_format(&self.adapter).unwrap();
        let sc_desc = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: swapchain_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Immediate,
        };
        self.surface.configure(&self.device, &sc_desc);
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn async_pool(&self) -> &ThreadPool {
        &self.async_pool
    }
}
