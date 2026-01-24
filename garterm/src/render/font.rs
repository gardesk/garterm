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
    /// Fallback fonts for missing glyphs (symbols, icons, etc.)
    fallback_fonts: Vec<Arc<Font>>,
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

        // Load fallback fonts for symbols, icons, box drawing, etc.
        let fallback_fonts = Self::load_fallback_fonts();

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
            fallback_fonts,
            size,
            cell_width,
            cell_height,
            baseline,
        })
    }

    /// Load fallback fonts for symbols and missing glyphs
    fn load_fallback_fonts() -> Vec<Arc<Font>> {
        // Build fallback paths including user fonts
        let mut fallback_paths: Vec<std::path::PathBuf> = Vec::new();

        // Add user font directories first (higher priority)
        if let Some(home) = dirs::home_dir() {
            let user_fonts = home.join(".local/share/fonts");
            fallback_paths.push(user_fonts.join("NerdFontsSymbols/SymbolsNerdFontMono-Regular.ttf"));
            fallback_paths.push(user_fonts.join("NerdFontsSymbols/SymbolsNerdFont-Regular.ttf"));
        }

        // System paths
        let system_paths = [
            // Nerd Fonts (icons, powerline symbols)
            "/usr/share/fonts/TTF/SymbolsNerdFont-Regular.ttf",
            "/usr/share/fonts/TTF/SymbolsNerdFontMono-Regular.ttf",
            // Noto fonts (broad Unicode coverage)
            "/usr/share/fonts/google-noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/google-noto-vf/NotoSansSymbols[wght].ttf",
            // Symbola (excellent Unicode coverage)
            "/usr/share/fonts/gdouros-symbola/Symbola.ttf",
            // DejaVu (good Unicode coverage including box drawing)
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            // Noto Sans (general fallback)
            "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
            // Noto Emoji
            "/usr/share/fonts/google-noto-emoji-fonts/NotoEmoji-Regular.ttf",
            "/usr/share/fonts/google-noto-color-emoji-fonts/NotoColorEmoji.ttf",
        ];
        fallback_paths.extend(system_paths.iter().map(std::path::PathBuf::from));

        let mut fallbacks = Vec::new();
        for path in &fallback_paths {
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = Font::from_bytes(data, FontSettings::default()) {
                    tracing::debug!("Loaded fallback font: {}", path.display());
                    fallbacks.push(Arc::new(font));
                }
            }
        }

        if fallbacks.is_empty() {
            tracing::warn!("No fallback fonts loaded - some symbols may not render");
        } else {
            tracing::info!("Loaded {} fallback fonts", fallbacks.len());
        }

        fallbacks
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

    /// Create a new FontCache with a different size (reuses loaded fonts)
    pub fn with_size(&self, new_size: f32) -> Self {
        // Recalculate cell metrics with new size
        let regular = self.fonts.get(&FontStyle::Regular).unwrap();
        let metrics = regular.metrics('M', new_size);
        let line_metrics = regular.horizontal_line_metrics(new_size);

        let cell_width = metrics.advance_width.ceil();
        let cell_height = line_metrics
            .map(|m| (m.ascent - m.descent + m.line_gap).ceil())
            .unwrap_or(new_size * 1.2);
        let baseline = line_metrics.map(|m| m.ascent).unwrap_or(new_size * 0.8);

        Self {
            fonts: self.fonts.clone(),
            fallback_fonts: self.fallback_fonts.clone(),
            size: new_size,
            cell_width,
            cell_height,
            baseline,
        }
    }

    /// Get font for style
    pub fn font(&self, style: FontStyle) -> &Font {
        self.fonts.get(&style).unwrap_or_else(|| {
            self.fonts.get(&FontStyle::Regular).unwrap()
        })
    }

    /// Rasterize a character, using fallback fonts if needed
    pub fn rasterize(&self, c: char, style: FontStyle) -> (fontdue::Metrics, Vec<u8>) {
        let primary_font = self.font(style);

        // Check if primary font has this glyph (glyph_index 0 means missing)
        if primary_font.lookup_glyph_index(c) != 0 {
            return primary_font.rasterize(c, self.size);
        }

        // Try fallback fonts
        for fallback in &self.fallback_fonts {
            if fallback.lookup_glyph_index(c) != 0 {
                return fallback.rasterize(c, self.size);
            }
        }

        // No font has this glyph - return from primary (will be placeholder/tofu)
        primary_font.rasterize(c, self.size)
    }

    /// Check if any font can render this character
    pub fn has_glyph(&self, c: char) -> bool {
        let primary = self.font(FontStyle::Regular);
        if primary.lookup_glyph_index(c) != 0 {
            return true;
        }
        self.fallback_fonts.iter().any(|f| f.lookup_glyph_index(c) != 0)
    }
}
