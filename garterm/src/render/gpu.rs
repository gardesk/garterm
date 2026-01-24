use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::ptr::NonNull;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("no suitable adapter found")]
    NoAdapter,
    #[error("failed to request device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),
    #[error("failed to create surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("surface error: {0}")]
    Surface(#[from] wgpu::SurfaceError),
}

/// Wrapper for X11 window handles for wgpu using Xcb (x11rb uses xcb internally)
pub struct XcbWindowHandle {
    window: u32,
    connection: *mut std::ffi::c_void,
    screen: i32,
}

// SAFETY: The xcb connection pointer is thread-safe for read operations
// and we only use it to create the wgpu surface.
unsafe impl Send for XcbWindowHandle {}
unsafe impl Sync for XcbWindowHandle {}

impl XcbWindowHandle {
    pub fn new(window: u32, connection: *mut std::ffi::c_void, screen: i32) -> Self {
        Self { window, connection, screen }
    }
}

impl HasWindowHandle for XcbWindowHandle {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = raw_window_handle::XcbWindowHandle::new(std::num::NonZeroU32::new(self.window).unwrap());
        let raw = RawWindowHandle::Xcb(handle);
        // SAFETY: The window handle is valid for the lifetime of this struct
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for XcbWindowHandle {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        let conn_ptr = NonNull::new(self.connection);
        let handle = raw_window_handle::XcbDisplayHandle::new(conn_ptr, self.screen);
        let raw = RawDisplayHandle::Xcb(handle);
        // SAFETY: The display handle is valid for the lifetime of this struct
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(raw) })
    }
}

/// GPU rendering context
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
}

impl GpuContext {
    /// Create a new GPU context for an X11 window
    ///
    /// Note: x11rb's RustConnection doesn't provide a raw xcb connection pointer,
    /// so we use a null pointer and rely on wgpu's Vulkan backend (which doesn't
    /// need the xcb connection directly - it uses the window ID).
    pub async fn new(
        window: u32,
        screen: i32,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });

        // x11rb's RustConnection doesn't give us a raw xcb_connection_t pointer.
        // However, wgpu's Vulkan backend can work with just the window ID on X11.
        // We pass a null pointer for the connection, which works for Vulkan.
        let handle = XcbWindowHandle::new(window, std::ptr::null_mut(), screen);

        let surface = instance.create_surface(handle)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(GpuError::NoAdapter)?;

        tracing::info!("Using GPU adapter: {:?}", adapter.get_info().name);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("garterm"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &surface_config);

        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
        })
    }

    /// Resize the surface
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    /// Get surface format
    pub fn format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    /// Get current surface dimensions
    pub fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }
}
