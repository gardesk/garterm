mod atlas;
mod font;
mod gpu;

pub use font::{FontCache, FontStyle};
pub use gpu::{GpuContext, GpuError};

use crate::terminal::{CellColor, Terminal};
use atlas::{GlyphAtlas, GlyphKey};
use bytemuck::{Pod, Zeroable};

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
    /// Default color palette (base 16)
    palette: [[f32; 4]; 16],
}

impl Renderer {
    /// Create a new renderer
    pub async fn new(
        window: u32,
        screen: i32,
        width: u32,
        height: u32,
        font_size: f32,
    ) -> Result<Self, GpuError> {
        let gpu = GpuContext::new(window, screen, width, height).await?;
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
            palette: default_palette(),
        })
    }

    /// Get cell dimensions
    pub fn cell_size(&self) -> (f32, f32) {
        self.fonts.cell_size()
    }

    /// Resize the renderer
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
    }

    /// Render the terminal
    pub fn render(&mut self, terminal: &Terminal) -> Result<(), GpuError> {
        // Update atlas if dirty
        if self.atlas.is_dirty() {
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
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.1,
                            b: 0.12,
                            a: 1.0,
                        }),
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

        Ok(())
    }

    fn build_vertices(&mut self, terminal: &Terminal) {
        self.vertices.clear();
        self.indices.clear();

        let (cell_w, cell_h) = self.fonts.cell_size();
        let (surface_w, surface_h) = self.gpu.size();
        let atlas_size = self.atlas.size();

        // Convert pixel to NDC
        let to_ndc_x = |x: f32| (x / surface_w as f32) * 2.0 - 1.0;
        let to_ndc_y = |y: f32| 1.0 - (y / surface_h as f32) * 2.0;

        let grid = terminal.grid();
        let cols = terminal.cols();
        let rows = terminal.rows();

        // Render each cell
        for row in 0..rows {
            if let Some(line) = grid.line(row) {
                for col in 0..cols {
                    let cell = &line[col];

                    let x = col as f32 * cell_w;
                    let y = row as f32 * cell_h;

                    // Background (if not default)
                    if cell.bg != CellColor::Default {
                        let bg_color = self.color_to_rgba(&cell.bg, false);
                        self.add_quad(
                            to_ndc_x(x), to_ndc_y(y),
                            to_ndc_x(x + cell_w), to_ndc_y(y + cell_h),
                            0.0, 0.0, 0.0, 0.0, // No UV for solid
                            bg_color,
                            0.0, // Not a glyph
                        );
                    }

                    // Character (if not space)
                    if cell.c != ' ' {
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
                                let fg_color = self.color_to_rgba(&cell.fg, true);

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
                }
            }
        }

        // Render cursor
        let cursor = terminal.cursor();
        if terminal.modes().cursor_visible {
            let x = cursor.col as f32 * cell_w;
            let y = cursor.row as f32 * cell_h;
            let cursor_color = [0.8, 0.8, 0.8, 1.0];

            self.add_quad(
                to_ndc_x(x), to_ndc_y(y),
                to_ndc_x(x + cell_w), to_ndc_y(y + cell_h),
                0.0, 0.0, 0.0, 0.0,
                cursor_color,
                0.0,
            );
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
                    [0.8, 0.8, 0.85, 1.0] // Default foreground
                } else {
                    [0.1, 0.1, 0.12, 1.0] // Default background
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

/// Default color palette (roughly xterm-256color base 16)
fn default_palette() -> [[f32; 4]; 16] {
    [
        [0.0, 0.0, 0.0, 1.0],       // 0: Black
        [0.8, 0.0, 0.0, 1.0],       // 1: Red
        [0.0, 0.8, 0.0, 1.0],       // 2: Green
        [0.8, 0.8, 0.0, 1.0],       // 3: Yellow
        [0.0, 0.0, 0.8, 1.0],       // 4: Blue
        [0.8, 0.0, 0.8, 1.0],       // 5: Magenta
        [0.0, 0.8, 0.8, 1.0],       // 6: Cyan
        [0.75, 0.75, 0.75, 1.0],    // 7: White
        [0.5, 0.5, 0.5, 1.0],       // 8: Bright Black
        [1.0, 0.0, 0.0, 1.0],       // 9: Bright Red
        [0.0, 1.0, 0.0, 1.0],       // 10: Bright Green
        [1.0, 1.0, 0.0, 1.0],       // 11: Bright Yellow
        [0.0, 0.0, 1.0, 1.0],       // 12: Bright Blue
        [1.0, 0.0, 1.0, 1.0],       // 13: Bright Magenta
        [0.0, 1.0, 1.0, 1.0],       // 14: Bright Cyan
        [1.0, 1.0, 1.0, 1.0],       // 15: Bright White
    ]
}
