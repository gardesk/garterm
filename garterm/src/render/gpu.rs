use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::ptr::NonNull;
use thiserror::Error;
use x11_dl::xlib::Xlib;

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
    #[error("failed to open X11 display")]
    X11Display,
    #[error("failed to load xlib")]
    XlibLoad,
}

/// Xlib display connection for wgpu
pub struct XlibDisplay {
    xlib: Xlib,
    display: *mut x11_dl::xlib::Display,
}

impl XlibDisplay {
    pub fn open() -> Result<Self, GpuError> {
        let xlib = Xlib::open().map_err(|_| GpuError::XlibLoad)?;
        let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
        if display.is_null() {
            return Err(GpuError::X11Display);
        }
        Ok(Self { xlib, display })
    }

    pub fn display_ptr(&self) -> *mut std::ffi::c_void {
        self.display as *mut _
    }

    pub fn default_screen(&self) -> i32 {
        unsafe { (self.xlib.XDefaultScreen)(self.display) }
    }

    /// Flush pending X11 requests to ensure frames are displayed
    pub fn flush(&self) {
        unsafe {
            (self.xlib.XFlush)(self.display);
        }
    }

    /// Sync with X server (blocking)
    pub fn sync(&self) {
        unsafe {
            (self.xlib.XSync)(self.display, 0);
        }
    }
}

impl Drop for XlibDisplay {
    fn drop(&mut self) {
        unsafe {
            (self.xlib.XCloseDisplay)(self.display);
        }
    }
}

// SAFETY: Xlib display can be shared between threads
unsafe impl Send for XlibDisplay {}
unsafe impl Sync for XlibDisplay {}

/// Wrapper for X11 window handles for wgpu using Xlib
pub struct XlibWindowHandle {
    window: u32,
    display: *mut std::ffi::c_void,
    screen: i32,
}

// SAFETY: The Xlib display pointer is thread-safe for read operations
// and we only use it to create the wgpu surface.
unsafe impl Send for XlibWindowHandle {}
unsafe impl Sync for XlibWindowHandle {}

impl XlibWindowHandle {
    pub fn new(window: u32, display: *mut std::ffi::c_void, screen: i32) -> Self {
        Self { window, display, screen }
    }
}

impl HasWindowHandle for XlibWindowHandle {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = raw_window_handle::XlibWindowHandle::new(self.window as _);
        let raw = RawWindowHandle::Xlib(handle);
        // SAFETY: The window handle is valid for the lifetime of this struct
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for XlibWindowHandle {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        let display_ptr = NonNull::new(self.display);
        let handle = raw_window_handle::XlibDisplayHandle::new(display_ptr, self.screen);
        let raw = RawDisplayHandle::Xlib(handle);
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
    // Keep xlib display alive for the lifetime of the surface
    xlib_display: XlibDisplay,
}

impl GpuContext {
    /// Create a new GPU context for an X11 window
    pub async fn new(
        window: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        // Open Xlib display for wgpu
        let xlib_display = XlibDisplay::open()?;
        let display = xlib_display.display_ptr();
        let screen = xlib_display.default_screen();

        // Try Vulkan first (higher texture limits), fall back to GL
        // GL has a 2048 texture limit on some systems which breaks full-screen windows
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });

        let handle = XlibWindowHandle::new(window, display, screen);

        let surface = instance.create_surface(handle)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(GpuError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        tracing::info!("Using GPU adapter: {:?} (backend: {:?})", adapter_info.name, adapter_info.backend);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("garterm"),
                    required_features: wgpu::Features::empty(),
                    // Use default limits (8192 max texture) instead of downlevel (2048)
                    // to support full-screen windows on high-res displays
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        tracing::info!("Available alpha modes: {:?}", surface_caps.alpha_modes);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        // Prefer opaque alpha mode to avoid compositing issues
        let alpha_mode = if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            surface_caps.alpha_modes[0]
        };
        tracing::info!("Using alpha mode: {:?}", alpha_mode);

        // Use Mailbox if available for lower latency, fall back to Fifo
        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            tracing::info!("Using Mailbox present mode");
            wgpu::PresentMode::Mailbox
        } else {
            tracing::info!("Using Fifo present mode");
            wgpu::PresentMode::Fifo
        };

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 1, // Reduce latency
        };

        surface.configure(&device, &surface_config);

        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
            xlib_display,
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

    /// Sync with X11 server to ensure frames are displayed
    pub fn sync_display(&self) {
        self.xlib_display.sync();
    }
}
