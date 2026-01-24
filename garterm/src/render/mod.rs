mod atlas;
mod font;
mod gpu;

pub use font::{FontCache, FontStyle};
pub use gpu::{GpuContext, GpuError};

use crate::config::ColorPalette;
use crate::terminal::{CellColor, Terminal, UnderlineStyle};
use crate::ui::{TabBarRenderData, TabRect};
use atlas::{GlyphAtlas, GlyphKey};
use bytemuck::{Pod, Zeroable};

/// Selection bounds for rendering
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectionBounds {
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
    pub is_block: bool,
}

impl SelectionBounds {
    /// Check if a cell is within this selection
    pub fn contains(&self, row: usize, col: usize, _cols: usize) -> bool {
        if self.is_block {
            // Block selection: rectangular
            let (min_col, max_col) = if self.start_col <= self.end_col {
                (self.start_col, self.end_col)
            } else {
                (self.end_col, self.start_col)
            };
            row >= self.start_row && row <= self.end_row && col >= min_col && col <= max_col
        } else {
            // Normal selection: continuous from start to end
            if row < self.start_row || row > self.end_row {
                return false;
            }
            if row == self.start_row && row == self.end_row {
                col >= self.start_col && col <= self.end_col
            } else if row == self.start_row {
                col >= self.start_col
            } else if row == self.end_row {
                col <= self.end_col
            } else {
                true // Middle lines fully selected
            }
        }
    }
}

/// Information needed to render a pane at a specific position
pub struct PaneRenderInfo<'a> {
    pub terminal: &'a Terminal,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    /// Optional selection bounds (only for focused pane)
    pub selection: Option<SelectionBounds>,
    /// Optional search info for highlighting matches
    pub search: Option<SearchRenderInfo<'a>>,
}

/// Search rendering information for highlighting in pane
pub struct SearchRenderInfo<'a> {
    /// Search query to display
    pub query: &'a str,
    /// All matches to highlight (visible row, col_start, col_end)
    pub matches: &'a [(usize, usize, usize)],
    /// Current match index to highlight specially
    pub current_match_idx: Option<usize>,
    /// Match count text (e.g., "3/15")
    pub match_count: &'a str,
    /// Whether search input is active
    pub active: bool,
    /// Case insensitive flag
    pub case_insensitive: bool,
}

/// Search overlay for the search bar at the bottom of the screen
pub struct SearchOverlay<'a> {
    /// Current search query
    pub query: &'a str,
    /// Match count text (e.g., "3/15" or "No matches")
    pub match_count: &'a str,
    /// Whether search input is active (cursor visible)
    pub active: bool,
    /// Case insensitive mode
    pub case_insensitive: bool,
}

/// Vertex for rendering quads (glyphs and backgrounds)
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    is_glyph: f32, // 1.0 for glyph, 0.0 for solid
}

impl Vertex {
    const ATTRS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Float32x4,
        3 => Float32,
    ];

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

/// Terminal renderer
pub struct Renderer {
    gpu: GpuContext,
    fonts: FontCache,
    atlas: GlyphAtlas,
    atlas_texture: wgpu::Texture,
    atlas_bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    /// Color palette from config
    colors: ColorPalette,
    /// Base 16 palette for rendering
    palette: [[f32; 4]; 16],
}

impl Renderer {
    /// Create a new renderer
    pub async fn new(
        window: u32,
        width: u32,
        height: u32,
        font_size: f32,
        colors: ColorPalette,
    ) -> Result<Self, GpuError> {
        let gpu = GpuContext::new(window, width, height).await?;
        let fonts = FontCache::new(font_size).expect("Failed to load fonts");

        // Create glyph atlas
        let atlas = GlyphAtlas::new(1024, 1024);
        let atlas_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_atlas"),
            size: wgpu::Extent3d {
                width: 1024,
                height: 1024,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Create bind group layout and bind group
        let bind_group_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let atlas_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        // Create shader
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terminal_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Create pipeline
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terminal_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terminal_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.format(),
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create vertex/index buffers
        let vertex_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertex_buffer"),
            size: 1024 * 1024, // 1MB
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("index_buffer"),
            size: 512 * 1024, // 512KB
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let palette = colors.to_render_palette();

        Ok(Self {
            gpu,
            fonts,
            atlas,
            atlas_texture,
            atlas_bind_group,
            pipeline,
            vertex_buffer,
            index_buffer,
            vertices: Vec::new(),
            indices: Vec::new(),
            colors,
            palette,
        })
    }

    /// Get cell dimensions
    pub fn cell_size(&self) -> (f32, f32) {
        self.fonts.cell_size()
    }

    /// Get current surface size
    pub fn size(&self) -> (u32, u32) {
        self.gpu.size()
    }

    /// Resize the renderer
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
    }

    /// Update color palette (for hot reload)
    pub fn set_colors(&mut self, colors: ColorPalette) {
        self.palette = colors.to_render_palette();
        self.colors = colors;
    }

    /// Change font size (clears glyph atlas)
    pub fn set_font_size(&mut self, size: f32) {
        self.fonts = self.fonts.with_size(size);
        self.atlas.clear();
    }

    /// Get current font size
    pub fn font_size(&self) -> f32 {
        self.fonts.size()
    }

    /// Render the terminal
    pub fn render(&mut self, terminal: &Terminal) -> Result<(), GpuError> {
        // Update atlas if dirty
        if self.atlas.is_dirty() {
            tracing::debug!("Uploading atlas texture");
            self.gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                self.atlas.data(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.atlas.size().0),
                    rows_per_image: Some(self.atlas.size().1),
                },
                wgpu::Extent3d {
                    width: self.atlas.size().0,
                    height: self.atlas.size().1,
                    depth_or_array_layers: 1,
                },
            );
            self.atlas.clear_dirty();
        }

        // Build vertex data
        self.build_vertices(terminal);

        tracing::debug!(
            "Rendering: {} vertices, {} indices, surface {}x{}",
            self.vertices.len(),
            self.indices.len(),
            self.gpu.size().0,
            self.gpu.size().1
        );

        // Upload vertex data
        self.gpu.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        self.gpu.queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&self.indices));

        // Get surface texture
        let output = self.gpu.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terminal_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.colors.background.to_wgpu_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.atlas_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..self.indices.len() as u32, 0, 0..1);
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        // Poll device to ensure GPU work is complete
        self.gpu.device.poll(wgpu::Maintain::Wait);

        // Sync with X11 server to ensure the frame is displayed
        self.gpu.sync_display();

        Ok(())
    }

    /// Render the full scene: tab bar + all panes + optional search overlay
    pub fn render_scene(
        &mut self,
        tab_bar: &TabBarRenderData,
        panes: &[PaneRenderInfo<'_>],
    ) -> Result<(), GpuError> {
        self.render_scene_with_search(tab_bar, panes, None)
    }

    /// Render the full scene with optional search overlay
    pub fn render_scene_with_search(
        &mut self,
        tab_bar: &TabBarRenderData,
        panes: &[PaneRenderInfo<'_>],
        search: Option<&SearchOverlay>,
    ) -> Result<(), GpuError> {
        // Update atlas if dirty
        if self.atlas.is_dirty() {
            tracing::debug!("Uploading atlas texture");
            self.gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                self.atlas.data(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.atlas.size().0),
                    rows_per_image: Some(self.atlas.size().1),
                },
                wgpu::Extent3d {
                    width: self.atlas.size().0,
                    height: self.atlas.size().1,
                    depth_or_array_layers: 1,
                },
            );
            self.atlas.clear_dirty();
        }

        // Build vertex data
        self.vertices.clear();
        self.indices.clear();

        // Build tab bar vertices first (at top)
        self.build_tab_bar(tab_bar);

        // Build pane vertices
        for (i, pane) in panes.iter().enumerate() {
            self.build_vertices_with_selection(
                pane.terminal,
                pane.x,
                pane.y,
                false,
                pane.selection.as_ref(),
            );

            if panes.len() > 1 {
                self.add_pane_border(pane.x, pane.y, pane.width, pane.height, pane.focused);
            }

            tracing::trace!(
                "Pane {} at ({}, {}) size {}x{} focused={}",
                i, pane.x, pane.y, pane.width, pane.height, pane.focused
            );
        }

        // Build search overlay if active
        if let Some(search) = search {
            self.build_search_overlay(search);
        }

        tracing::debug!(
            "Rendering {} panes + tab bar: {} vertices, {} indices",
            panes.len(),
            self.vertices.len(),
            self.indices.len()
        );

        // Upload and render
        self.gpu.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        self.gpu.queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&self.indices));

        let output = self.gpu.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terminal_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.colors.background.to_wgpu_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.atlas_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..self.indices.len() as u32, 0, 0..1);
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        self.gpu.device.poll(wgpu::Maintain::Wait);
        self.gpu.sync_display();

        Ok(())
    }

    /// Build vertices for the tab bar
    fn build_tab_bar(&mut self, data: &TabBarRenderData) {
        let (surface_w, surface_h) = self.gpu.size();
        let to_ndc_x = |x: f32| (x / surface_w as f32) * 2.0 - 1.0;
        let to_ndc_y = |y: f32| 1.0 - (y / surface_h as f32) * 2.0;

        // Tab bar background
        if let Some(ref bg) = data.background {
            self.add_quad(
                to_ndc_x(bg.x), to_ndc_y(bg.y),
                to_ndc_x(bg.x + bg.width), to_ndc_y(bg.y + bg.height),
                0.0, 0.0, 0.0, 0.0,
                bg.color,
                0.0,
            );
        }

        // Each tab
        for tab in &data.tabs {
            // Tab background
            self.add_quad(
                to_ndc_x(tab.rect.x), to_ndc_y(tab.rect.y),
                to_ndc_x(tab.rect.x + tab.rect.width), to_ndc_y(tab.rect.y + tab.rect.height),
                0.0, 0.0, 0.0, 0.0,
                tab.rect.color,
                0.0,
            );

            // Tab title text (render each character using configured color)
            self.render_text_at(
                &tab.title,
                tab.title_x,
                tab.title_y + self.fonts.baseline(),
                tab.fg_color,
            );
        }
    }

    /// Render text at a specific pixel position
    fn render_text_at(&mut self, text: &str, start_x: f32, start_y: f32, color: [f32; 4]) {
        let (surface_w, surface_h) = self.gpu.size();
        let (cell_w, _cell_h) = self.fonts.cell_size();
        let atlas_size = self.atlas.size();

        let to_ndc_x = |x: f32| (x / surface_w as f32) * 2.0 - 1.0;
        let to_ndc_y = |y: f32| 1.0 - (y / surface_h as f32) * 2.0;

        let mut x = start_x;
        for c in text.chars() {
            let key = GlyphKey { c, style: FontStyle::Regular };
            if let Some(entry) = self.atlas.get_or_insert(key, &self.fonts) {
                if entry.width > 0 && entry.height > 0 {
                    let glyph_x = x + entry.bearing_x as f32;
                    let glyph_y = start_y - entry.bearing_y as f32 - entry.height as f32;

                    let u0 = entry.x as f32 / atlas_size.0 as f32;
                    let v0 = entry.y as f32 / atlas_size.1 as f32;
                    let u1 = (entry.x + entry.width) as f32 / atlas_size.0 as f32;
                    let v1 = (entry.y + entry.height) as f32 / atlas_size.1 as f32;

                    self.add_quad(
                        to_ndc_x(glyph_x), to_ndc_y(glyph_y),
                        to_ndc_x(glyph_x + entry.width as f32), to_ndc_y(glyph_y + entry.height as f32),
                        u0, v0, u1, v1,
                        color,
                        1.0,
                    );
                }
            }
            x += cell_w;
        }
    }

    /// Build search overlay bar at the bottom of the screen
    fn build_search_overlay(&mut self, search: &SearchOverlay) {
        let (surface_w, surface_h) = self.gpu.size();
        let (cell_w, cell_h) = self.fonts.cell_size();
        let to_ndc_x = |x: f32| (x / surface_w as f32) * 2.0 - 1.0;
        let to_ndc_y = |y: f32| 1.0 - (y / surface_h as f32) * 2.0;

        // Bar height: one cell height + padding
        let bar_height = cell_h + 8.0;
        let bar_y = surface_h as f32 - bar_height;

        // Background (dark semi-transparent bar)
        let bg_color = [0.15, 0.15, 0.15, 0.95];
        self.add_quad(
            to_ndc_x(0.0), to_ndc_y(bar_y),
            to_ndc_x(surface_w as f32), to_ndc_y(surface_h as f32),
            0.0, 0.0, 0.0, 0.0,
            bg_color,
            0.0,
        );

        // Border at top of bar
        let border_color = [0.4, 0.6, 1.0, 1.0];
        self.add_quad(
            to_ndc_x(0.0), to_ndc_y(bar_y),
            to_ndc_x(surface_w as f32), to_ndc_y(bar_y + 1.0),
            0.0, 0.0, 0.0, 0.0,
            border_color,
            0.0,
        );

        // Text position
        let text_y = bar_y + 4.0 + cell_h; // Baseline position
        let mut x = 8.0;

        // "Search: " prefix
        let prefix = if search.case_insensitive { "Search (i): " } else { "Search: " };
        let prefix_color = [0.7, 0.7, 0.7, 1.0];
        self.render_text_at(prefix, x, text_y, prefix_color);
        x += prefix.len() as f32 * cell_w;

        // Query text
        let query_color = [1.0, 1.0, 1.0, 1.0];
        self.render_text_at(search.query, x, text_y, query_color);
        x += search.query.len() as f32 * cell_w;

        // Cursor (blinking block when active)
        if search.active {
            let cursor_color = [0.8, 0.8, 0.8, 0.8];
            self.add_quad(
                to_ndc_x(x), to_ndc_y(bar_y + 4.0),
                to_ndc_x(x + cell_w), to_ndc_y(bar_y + 4.0 + cell_h),
                0.0, 0.0, 0.0, 0.0,
                cursor_color,
                0.0,
            );
        }

        // Match count on the right
        if !search.match_count.is_empty() {
            let count_color = [0.6, 0.8, 0.6, 1.0];
            let count_x = surface_w as f32 - (search.match_count.len() as f32 * cell_w) - 8.0;
            self.render_text_at(search.match_count, count_x, text_y, count_color);
        }
    }

    /// Render multiple panes at their positions
    pub fn render_panes(&mut self, panes: &[PaneRenderInfo<'_>]) -> Result<(), GpuError> {
        // Update atlas if dirty
        if self.atlas.is_dirty() {
            tracing::debug!("Uploading atlas texture");
            self.gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                self.atlas.data(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.atlas.size().0),
                    rows_per_image: Some(self.atlas.size().1),
                },
                wgpu::Extent3d {
                    width: self.atlas.size().0,
                    height: self.atlas.size().1,
                    depth_or_array_layers: 1,
                },
            );
            self.atlas.clear_dirty();
        }

        // Build vertex data for all panes
        self.vertices.clear();
        self.indices.clear();

        for (i, pane) in panes.iter().enumerate() {
            // Build vertices for this pane at its position
            self.build_vertices_with_selection(
                pane.terminal,
                pane.x,
                pane.y,
                false,
                pane.selection.as_ref(),
            );

            // Draw a subtle border around non-focused panes (or highlight focused)
            if panes.len() > 1 {
                self.add_pane_border(pane.x, pane.y, pane.width, pane.height, pane.focused);
            }

            // Show scroll indicator when scrolled back from bottom
            if pane.terminal.is_scrolled() {
                self.add_scroll_indicator(pane.x, pane.y, pane.width, pane.height);
            }

            tracing::trace!(
                "Pane {} at ({}, {}) size {}x{} focused={}",
                i, pane.x, pane.y, pane.width, pane.height, pane.focused
            );
        }

        tracing::debug!(
            "Rendering {} panes: {} vertices, {} indices",
            panes.len(),
            self.vertices.len(),
            self.indices.len()
        );

        // Upload vertex data
        self.gpu.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        self.gpu.queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&self.indices));

        // Get surface texture
        let output = self.gpu.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terminal_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.colors.background.to_wgpu_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.atlas_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..self.indices.len() as u32, 0, 0..1);
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        self.gpu.device.poll(wgpu::Maintain::Wait);
        self.gpu.sync_display();

        Ok(())
    }

    /// Add a border around a pane
    fn add_pane_border(&mut self, x: u32, y: u32, width: u32, height: u32, focused: bool) {
        let (surface_w, surface_h) = self.gpu.size();
        let to_ndc_x = |px: f32| (px / surface_w as f32) * 2.0 - 1.0;
        let to_ndc_y = |py: f32| 1.0 - (py / surface_h as f32) * 2.0;

        let border_width = 1.0;
        let color = if focused {
            [0.4, 0.6, 1.0, 1.0] // Blue-ish for focused
        } else {
            [0.3, 0.3, 0.3, 1.0] // Gray for unfocused
        };

        let x = x as f32;
        let y = y as f32;
        let w = width as f32;
        let h = height as f32;

        // Top border
        self.add_quad(
            to_ndc_x(x), to_ndc_y(y),
            to_ndc_x(x + w), to_ndc_y(y + border_width),
            0.0, 0.0, 0.0, 0.0,
            color,
            0.0,
        );

        // Bottom border
        self.add_quad(
            to_ndc_x(x), to_ndc_y(y + h - border_width),
            to_ndc_x(x + w), to_ndc_y(y + h),
            0.0, 0.0, 0.0, 0.0,
            color,
            0.0,
        );

        // Left border
        self.add_quad(
            to_ndc_x(x), to_ndc_y(y),
            to_ndc_x(x + border_width), to_ndc_y(y + h),
            0.0, 0.0, 0.0, 0.0,
            color,
            0.0,
        );

        // Right border
        self.add_quad(
            to_ndc_x(x + w - border_width), to_ndc_y(y),
            to_ndc_x(x + w), to_ndc_y(y + h),
            0.0, 0.0, 0.0, 0.0,
            color,
            0.0,
        );
    }

    /// Add a visual indicator when the terminal is scrolled up from the bottom.
    /// Shows a small down arrow or bar at the bottom-right to indicate more content below.
    fn add_scroll_indicator(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let (surface_w, surface_h) = self.gpu.size();
        let to_ndc_x = |px: f32| (px / surface_w as f32) * 2.0 - 1.0;
        let to_ndc_y = |py: f32| 1.0 - (py / surface_h as f32) * 2.0;

        // Indicator: small rounded rect at bottom-right corner
        // Use a bright color to make it noticeable
        let indicator_color = [0.8, 0.8, 0.3, 0.9]; // Yellow-ish

        let indicator_width = 24.0;
        let indicator_height = 6.0;
        let margin = 8.0;

        let ix = x as f32 + width as f32 - indicator_width - margin;
        let iy = y as f32 + height as f32 - indicator_height - margin;

        // Draw the indicator bar
        self.add_quad(
            to_ndc_x(ix), to_ndc_y(iy),
            to_ndc_x(ix + indicator_width), to_ndc_y(iy + indicator_height),
            0.0, 0.0, 0.0, 0.0,
            indicator_color,
            0.0,
        );

        // Draw a small down arrow (triangle) below/next to the bar
        // Using two small quads to form an arrow shape
        let arrow_size = 8.0;
        let arrow_x = ix + (indicator_width - arrow_size) / 2.0;
        let arrow_y = iy + indicator_height + 2.0;

        // Left half of arrow
        self.add_quad(
            to_ndc_x(arrow_x), to_ndc_y(arrow_y),
            to_ndc_x(arrow_x + arrow_size / 2.0), to_ndc_y(arrow_y + arrow_size / 2.0),
            0.0, 0.0, 0.0, 0.0,
            indicator_color,
            0.0,
        );

        // Right half of arrow
        self.add_quad(
            to_ndc_x(arrow_x + arrow_size / 2.0), to_ndc_y(arrow_y),
            to_ndc_x(arrow_x + arrow_size), to_ndc_y(arrow_y + arrow_size / 2.0),
            0.0, 0.0, 0.0, 0.0,
            indicator_color,
            0.0,
        );
    }

    fn build_vertices(&mut self, terminal: &Terminal) {
        self.build_vertices_with_selection(terminal, 0, 0, true, None);
    }

    /// Build vertices for a terminal at a specific offset, with optional selection highlighting
    fn build_vertices_with_selection(
        &mut self,
        terminal: &Terminal,
        offset_x: u32,
        offset_y: u32,
        clear: bool,
        selection: Option<&SelectionBounds>,
    ) {
        if clear {
            self.vertices.clear();
            self.indices.clear();
        }

        let (cell_w, cell_h) = self.fonts.cell_size();
        let (surface_w, surface_h) = self.gpu.size();
        let atlas_size = self.atlas.size();

        // Convert pixel to NDC
        let to_ndc_x = |x: f32| (x / surface_w as f32) * 2.0 - 1.0;
        let to_ndc_y = |y: f32| 1.0 - (y / surface_h as f32) * 2.0;

        let cols = terminal.cols();
        let rows = terminal.rows();

        // Selection highlight color (semi-transparent blue)
        let selection_bg = [0.3, 0.5, 0.8, 0.5];

        // Use visible_lines to account for scrollback
        for (row, line) in terminal.grid().visible_lines().enumerate().take(rows) {
            for col in 0..cols {
                let cell = &line[col];

                let x = offset_x as f32 + col as f32 * cell_w;
                let y = offset_y as f32 + row as f32 * cell_h;

                // Handle inverse (reverse video) attribute
                let inverse = cell.attrs.inverse;

                // Check if cell is selected
                let is_selected = selection
                    .map(|s| s.contains(row, col, cols))
                    .unwrap_or(false);

                // Background: selection takes priority, then cell background (or fg if inverse)
                if is_selected {
                    self.add_quad(
                        to_ndc_x(x), to_ndc_y(y),
                        to_ndc_x(x + cell_w), to_ndc_y(y + cell_h),
                        0.0, 0.0, 0.0, 0.0,
                        selection_bg,
                        0.0,
                    );
                } else if inverse {
                    // Inverse: background gets fg color (flip is_fg to get correct default)
                    let bg_color = self.color_to_rgba(&cell.fg, true);
                    self.add_quad(
                        to_ndc_x(x), to_ndc_y(y),
                        to_ndc_x(x + cell_w), to_ndc_y(y + cell_h),
                        0.0, 0.0, 0.0, 0.0,
                        bg_color,
                        0.0,
                    );
                } else if cell.bg != CellColor::Default {
                    let bg_color = self.color_to_rgba(&cell.bg, false);
                    self.add_quad(
                        to_ndc_x(x), to_ndc_y(y),
                        to_ndc_x(x + cell_w), to_ndc_y(y + cell_h),
                        0.0, 0.0, 0.0, 0.0,
                        bg_color,
                        0.0,
                    );
                }

                // Character (if not space or null)
                if cell.c != ' ' && cell.c != '\0' {
                    let style = if cell.attrs.bold && cell.attrs.italic {
                        FontStyle::BoldItalic
                    } else if cell.attrs.bold {
                        FontStyle::Bold
                    } else if cell.attrs.italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Regular
                    };

                    let key = GlyphKey { c: cell.c, style };
                    if let Some(entry) = self.atlas.get_or_insert(key, &self.fonts) {
                        if entry.width > 0 && entry.height > 0 {
                            // For selected text, use contrasting foreground
                            let fg_color = if is_selected {
                                [1.0, 1.0, 1.0, 1.0] // White text on selection
                            } else if inverse {
                                // Inverse: text gets bg color (use is_fg=false for correct default)
                                self.color_to_rgba(&cell.bg, false)
                            } else {
                                self.color_to_rgba(&cell.fg, true)
                            };

                            // Calculate glyph position
                            let glyph_x = x + entry.bearing_x as f32;
                            let glyph_y = y + self.fonts.baseline() - entry.bearing_y as f32 - entry.height as f32;

                            // UV coordinates in atlas
                            let u0 = entry.x as f32 / atlas_size.0 as f32;
                            let v0 = entry.y as f32 / atlas_size.1 as f32;
                            let u1 = (entry.x + entry.width) as f32 / atlas_size.0 as f32;
                            let v1 = (entry.y + entry.height) as f32 / atlas_size.1 as f32;

                            self.add_quad(
                                to_ndc_x(glyph_x), to_ndc_y(glyph_y),
                                to_ndc_x(glyph_x + entry.width as f32), to_ndc_y(glyph_y + entry.height as f32),
                                u0, v0, u1, v1,
                                fg_color,
                                1.0, // Is a glyph
                            );
                        }
                    }
                }

                // Underline rendering
                if cell.attrs.underline != UnderlineStyle::None {
                    let fg_color = if is_selected {
                        [1.0, 1.0, 1.0, 1.0]
                    } else if inverse {
                        self.color_to_rgba(&cell.bg, false)
                    } else {
                        self.color_to_rgba(&cell.fg, true)
                    };

                    // Position underline at baseline + 1 pixel
                    let underline_y = y + self.fonts.baseline() + 1.0;
                    let underline_thickness = (cell_h / 14.0).max(1.0); // Scale with font size

                    match cell.attrs.underline {
                        UnderlineStyle::Single => {
                            self.add_quad(
                                to_ndc_x(x), to_ndc_y(underline_y),
                                to_ndc_x(x + cell_w), to_ndc_y(underline_y + underline_thickness),
                                0.0, 0.0, 0.0, 0.0,
                                fg_color,
                                0.0,
                            );
                        }
                        UnderlineStyle::Double => {
                            // First line
                            self.add_quad(
                                to_ndc_x(x), to_ndc_y(underline_y),
                                to_ndc_x(x + cell_w), to_ndc_y(underline_y + underline_thickness),
                                0.0, 0.0, 0.0, 0.0,
                                fg_color,
                                0.0,
                            );
                            // Second line
                            self.add_quad(
                                to_ndc_x(x), to_ndc_y(underline_y + underline_thickness * 2.0),
                                to_ndc_x(x + cell_w), to_ndc_y(underline_y + underline_thickness * 3.0),
                                0.0, 0.0, 0.0, 0.0,
                                fg_color,
                                0.0,
                            );
                        }
                        UnderlineStyle::Curly | UnderlineStyle::Dotted | UnderlineStyle::Dashed => {
                            // For simplicity, render these as single underline for now
                            // TODO: Implement curly/dotted/dashed styles with proper rendering
                            self.add_quad(
                                to_ndc_x(x), to_ndc_y(underline_y),
                                to_ndc_x(x + cell_w), to_ndc_y(underline_y + underline_thickness),
                                0.0, 0.0, 0.0, 0.0,
                                fg_color,
                                0.0,
                            );
                        }
                        UnderlineStyle::None => {}
                    }
                }

                // Strikethrough rendering
                if cell.attrs.strikethrough {
                    let fg_color = if is_selected {
                        [1.0, 1.0, 1.0, 1.0]
                    } else if inverse {
                        self.color_to_rgba(&cell.bg, false)
                    } else {
                        self.color_to_rgba(&cell.fg, true)
                    };

                    // Position strikethrough at middle of cell (roughly at x-height)
                    let strike_y = y + cell_h * 0.45;
                    let strike_thickness = (cell_h / 14.0).max(1.0);

                    self.add_quad(
                        to_ndc_x(x), to_ndc_y(strike_y),
                        to_ndc_x(x + cell_w), to_ndc_y(strike_y + strike_thickness),
                        0.0, 0.0, 0.0, 0.0,
                        fg_color,
                        0.0,
                    );
                }
            }
        }

        // Render cursor at correct viewport position
        // When scrolled, the cursor stays at its position in the active display,
        // but we need to offset it by the number of scrollback lines being shown
        let cursor = terminal.cursor();
        if terminal.modes().cursor_visible {
            let grid = terminal.grid();
            let scroll_offset = grid.scroll_offset();
            let scrollback_len = grid.scrollback_len();
            let rows = terminal.rows();

            // Calculate how many scrollback lines are visible in the viewport
            let scrollback_visible = scroll_offset.min(scrollback_len);

            // The cursor's visual row in the viewport
            // Active lines start after scrollback_visible lines
            let cursor_visual_row = scrollback_visible + cursor.row;

            // Only render if cursor is within the visible viewport
            if cursor_visual_row < rows {
                let x = offset_x as f32 + cursor.col as f32 * cell_w;
                let y = offset_y as f32 + cursor_visual_row as f32 * cell_h;
                let cursor_color = self.colors.cursor.to_rgba();

                self.add_quad(
                    to_ndc_x(x), to_ndc_y(y),
                    to_ndc_x(x + cell_w), to_ndc_y(y + cell_h),
                    0.0, 0.0, 0.0, 0.0,
                    cursor_color,
                    0.0,
                );
            }
        }
    }

    fn add_quad(
        &mut self,
        x0: f32, y0: f32,
        x1: f32, y1: f32,
        u0: f32, v0: f32,
        u1: f32, v1: f32,
        color: [f32; 4],
        is_glyph: f32,
    ) {
        let base = self.vertices.len() as u32;

        self.vertices.push(Vertex { position: [x0, y0], uv: [u0, v0], color, is_glyph });
        self.vertices.push(Vertex { position: [x1, y0], uv: [u1, v0], color, is_glyph });
        self.vertices.push(Vertex { position: [x1, y1], uv: [u1, v1], color, is_glyph });
        self.vertices.push(Vertex { position: [x0, y1], uv: [u0, v1], color, is_glyph });

        self.indices.extend_from_slice(&[
            base, base + 1, base + 2,
            base, base + 2, base + 3,
        ]);
    }

    fn color_to_rgba(&self, color: &CellColor, is_fg: bool) -> [f32; 4] {
        match color {
            CellColor::Default => {
                if is_fg {
                    self.colors.foreground.to_rgba()
                } else {
                    self.colors.background.to_rgba()
                }
            }
            CellColor::Indexed(idx) => {
                if (*idx as usize) < 16 {
                    self.palette[*idx as usize]
                } else if *idx < 232 {
                    // 216 color cube
                    let idx = *idx - 16;
                    let r = (idx / 36) % 6;
                    let g = (idx / 6) % 6;
                    let b = idx % 6;
                    [
                        if r > 0 { (r * 40 + 55) as f32 / 255.0 } else { 0.0 },
                        if g > 0 { (g * 40 + 55) as f32 / 255.0 } else { 0.0 },
                        if b > 0 { (b * 40 + 55) as f32 / 255.0 } else { 0.0 },
                        1.0,
                    ]
                } else {
                    // Grayscale
                    let level = (*idx - 232) * 10 + 8;
                    let v = level as f32 / 255.0;
                    [v, v, v, 1.0]
                }
            }
            CellColor::Rgb(r, g, b) => {
                [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0]
            }
        }
    }
}

