use fontdue::{Font, FontSettings};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FontError {
    #[error("failed to load font: {0}")]
    LoadFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Font variant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontStyle {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

/// Font cache for terminal rendering
pub struct FontCache {
    fonts: HashMap<FontStyle, Arc<Font>>,
    size: f32,
    cell_width: f32,
    cell_height: f32,
    baseline: f32,
}

impl FontCache {
    /// Load fonts from system or embedded fallback
    pub fn new(size: f32) -> Result<Self, FontError> {
        // Try to load system fonts, fall back to embedded
        let regular = Arc::new(Self::load_font(&[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/google-noto/NotoSansMono-Regular.ttf",
        ])?);

        let bold = Arc::new(Self::load_font(&[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        ])
        .unwrap_or_else(|_| (*regular).clone()));

        let italic = Arc::new(Self::load_font(&[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Italic.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono-Oblique.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Oblique.ttf",
        ])
        .unwrap_or_else(|_| (*regular).clone()));

        let bold_italic = Arc::new(Self::load_font(&[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-BoldItalic.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono-BoldOblique.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-BoldOblique.ttf",
        ])
        .unwrap_or_else(|_| (*bold).clone()));

        // Calculate cell metrics from regular font
        let metrics = regular.metrics('M', size);
        let line_metrics = regular.horizontal_line_metrics(size);

        let cell_width = metrics.advance_width.ceil();
        let cell_height = line_metrics
            .map(|m| (m.ascent - m.descent + m.line_gap).ceil())
            .unwrap_or(size * 1.2);
        let baseline = line_metrics.map(|m| m.ascent).unwrap_or(size * 0.8);

        let mut fonts = HashMap::new();
        fonts.insert(FontStyle::Regular, regular);
        fonts.insert(FontStyle::Bold, bold);
        fonts.insert(FontStyle::Italic, italic);
        fonts.insert(FontStyle::BoldItalic, bold_italic);

        Ok(Self {
            fonts,
            size,
            cell_width,
            cell_height,
            baseline,
        })
    }

    fn load_font(paths: &[&str]) -> Result<Font, FontError> {
        for path in paths {
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = Font::from_bytes(data, FontSettings::default()) {
                    tracing::info!("Loaded font: {}", path);
                    return Ok(font);
                }
            }
        }
        Err(FontError::LoadFailed("No suitable font found".into()))
    }

    /// Get cell dimensions
    pub fn cell_size(&self) -> (f32, f32) {
        (self.cell_width, self.cell_height)
    }

    /// Get baseline offset from top of cell
    pub fn baseline(&self) -> f32 {
        self.baseline
    }

    /// Get font size
    pub fn size(&self) -> f32 {
        self.size
    }

    /// Get font for style
    pub fn font(&self, style: FontStyle) -> &Font {
        self.fonts.get(&style).unwrap_or_else(|| {
            self.fonts.get(&FontStyle::Regular).unwrap()
        })
    }

    /// Rasterize a character
    pub fn rasterize(&self, c: char, style: FontStyle) -> (fontdue::Metrics, Vec<u8>) {
        let font = self.font(style);
        font.rasterize(c, self.size)
    }
}
