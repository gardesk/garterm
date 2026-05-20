use fontdue::{Font, FontSettings};
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
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

/// Lazily-loaded fallback fonts. Loading scans many paths and runs several
/// `fc-match` subprocesses (~150ms+), so we kick this off on a background
/// thread during startup. The first glyph that misses the primary font will
/// block on this if it hasn't finished yet — usually it has.
struct FallbackLoader {
    handle: Mutex<Option<JoinHandle<Vec<Arc<Font>>>>>,
    cache: OnceLock<Vec<Arc<Font>>>,
}

impl FallbackLoader {
    fn spawn() -> Arc<Self> {
        let handle = thread::Builder::new()
            .name("garterm-fallback-fonts".into())
            .spawn(load_fallback_fonts_impl)
            .ok();
        Arc::new(Self {
            handle: Mutex::new(handle),
            cache: OnceLock::new(),
        })
    }

    /// Get fallback fonts, joining the loader thread if not yet finished.
    fn get(&self) -> &[Arc<Font>] {
        if let Some(cached) = self.cache.get() {
            return cached;
        }
        let handle = self.handle.lock().unwrap().take();
        let fonts = handle
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        // It's fine if another thread won the race — both produced the same data.
        let _ = self.cache.set(fonts);
        self.cache.get().map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Font cache for terminal rendering
pub struct FontCache {
    fonts: HashMap<FontStyle, Arc<Font>>,
    /// Fallback fonts for missing glyphs (symbols, icons, etc.) — lazily loaded.
    fallback_loader: Arc<FallbackLoader>,
    /// Fonts discovered dynamically via fontconfig charset queries
    dynamic_fallbacks: Mutex<Vec<Arc<Font>>>,
    /// Codepoints we've already attempted fontconfig discovery for
    tried_codepoints: Mutex<HashSet<char>>,
    size: f32,
    cell_width: f32,
    cell_height: f32,
    baseline: f32,
}

impl FontCache {
    /// Resolve a font family + style to a file path using fontconfig (fc-match).
    fn fc_match(family: &str, style: &str) -> Option<String> {
        let query = format!("{}:style={}", family, style);
        Command::new("fc-match")
            .args(["-f", "%{file}", &query])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !path.is_empty() && std::path::Path::new(&path).exists() {
                        Some(path)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
    }

    /// Load fonts from system or embedded fallback
    pub fn new(family: &str, size: f32) -> Result<Self, FontError> {
        // Kick off fallback font loading on a background thread. These fonts are
        // only consulted when the primary font is missing a glyph, so the main
        // thread doesn't need to wait for them.
        let fallback_loader = FallbackLoader::spawn();

        // Hardcoded fallback paths for each style.
        const REGULAR_FALLBACKS: &[&str] = &[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/google-noto/NotoSansMono-Regular.ttf",
        ];
        const BOLD_FALLBACKS: &[&str] = &[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        ];
        const ITALIC_FALLBACKS: &[&str] = &[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Italic.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono-Oblique.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Oblique.ttf",
        ];
        const BOLD_ITALIC_FALLBACKS: &[&str] = &[
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-BoldItalic.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono-BoldOblique.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-BoldOblique.ttf",
        ];

        // Run the 4 fc-match queries in parallel; each spawns a subprocess
        // (~10-20ms apiece) and they're independent.
        let family_owned = family.to_string();
        let (regular, bold, italic, bold_italic) = thread::scope(|s| {
            let f = &family_owned;
            let r = s.spawn(|| Self::resolve_and_load(f, "Regular", REGULAR_FALLBACKS).ok());
            let b = s.spawn(|| Self::resolve_and_load(f, "Bold", BOLD_FALLBACKS).ok());
            let i = s.spawn(|| Self::resolve_and_load(f, "Italic", ITALIC_FALLBACKS).ok());
            let bi = s.spawn(|| Self::resolve_and_load(f, "Bold Italic", BOLD_ITALIC_FALLBACKS).ok());
            (
                r.join().ok().flatten(),
                b.join().ok().flatten(),
                i.join().ok().flatten(),
                bi.join().ok().flatten(),
            )
        });

        let regular = regular.map(Arc::new).ok_or_else(|| {
            FontError::LoadFailed("No suitable regular font found".into())
        })?;
        let bold = bold.map(Arc::new).unwrap_or_else(|| regular.clone());
        let italic = italic.map(Arc::new).unwrap_or_else(|| regular.clone());
        let bold_italic = bold_italic.map(Arc::new).unwrap_or_else(|| bold.clone());

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
            fallback_loader,
            dynamic_fallbacks: Mutex::new(Vec::new()),
            tried_codepoints: Mutex::new(HashSet::new()),
            size,
            cell_width,
            cell_height,
            baseline,
        })
    }

    /// Resolve a font path via fontconfig + hardcoded fallbacks and load it.
    fn resolve_and_load(family: &str, style: &str, fallbacks: &[&str]) -> Result<Font, FontError> {
        if let Some(p) = Self::fc_match(family, style) {
            if let Ok(data) = std::fs::read(&p) {
                if let Ok(font) = Font::from_bytes(data, FontSettings::default()) {
                    tracing::debug!("Loaded {}:{} from {}", family, style, p);
                    return Ok(font);
                }
            }
        }
        Self::load_font(fallbacks)
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
            fallback_loader: self.fallback_loader.clone(),
            dynamic_fallbacks: Mutex::new(self.dynamic_fallbacks.lock().unwrap().clone()),
            tried_codepoints: Mutex::new(self.tried_codepoints.lock().unwrap().clone()),
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

    /// Rasterize a character, using fallback fonts if needed.
    /// Falls back through static fallbacks, then dynamically-discovered fonts,
    /// and finally queries fontconfig for a font containing the codepoint.
    pub fn rasterize(&self, c: char, style: FontStyle) -> (fontdue::Metrics, Vec<u8>) {
        let primary_font = self.font(style);

        // Check if primary font has this glyph (glyph_index 0 means missing)
        if primary_font.lookup_glyph_index(c) != 0 {
            return primary_font.rasterize(c, self.size);
        }

        // Try static fallback fonts (waits for background loader if first miss)
        for fallback in self.fallback_loader.get() {
            if fallback.lookup_glyph_index(c) != 0 {
                return fallback.rasterize(c, self.size);
            }
        }

        // Try already-discovered dynamic fallbacks
        {
            let dynamic = self.dynamic_fallbacks.lock().unwrap();
            for fallback in dynamic.iter() {
                if fallback.lookup_glyph_index(c) != 0 {
                    return fallback.rasterize(c, self.size);
                }
            }
        }

        // Ask fontconfig to find a font for this codepoint (once per codepoint).
        // We hold the lock briefly to insert into tried_codepoints; the fc-list
        // subprocess runs without holding any locks.
        let first_try = {
            let mut tried = self.tried_codepoints.lock().unwrap();
            tried.insert(c)
        };
        if first_try {
            if let Some(font) = self.discover_font_for_char(c) {
                let result = font.rasterize(c, self.size);
                self.dynamic_fallbacks.lock().unwrap().push(font);
                return result;
            }
        }

        // No font has this glyph - return from primary (will be placeholder/tofu)
        primary_font.rasterize(c, self.size)
    }

    /// Query fontconfig for a font that contains the given character
    fn discover_font_for_char(&self, c: char) -> Option<Arc<Font>> {
        let codepoint = c as u32;
        let query = format!(":charset={:04x}", codepoint);

        let output = Command::new("fc-list")
            .args(["-f", "%{file}\n", &query])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut seen = HashSet::new();

        for line in stdout.lines() {
            let path = line.trim();
            if path.is_empty() || !seen.insert(path.to_string()) {
                continue;
            }

            // Skip color emoji fonts (fontdue can't rasterize bitmap/COLR fonts)
            if path.contains("ColorEmoji") {
                continue;
            }

            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = Font::from_bytes(data, FontSettings::default()) {
                    if font.lookup_glyph_index(c) != 0 {
                        tracing::debug!(
                            "Dynamic font fallback: U+{:04X} '{}' -> {}",
                            codepoint, c, path
                        );
                        return Some(Arc::new(font));
                    }
                }
            }
        }

        tracing::debug!("No font found for U+{:04X} '{}'", codepoint, c);
        None
    }

    /// Check if any font can render this character
    pub fn has_glyph(&self, c: char) -> bool {
        let primary = self.font(FontStyle::Regular);
        if primary.lookup_glyph_index(c) != 0 {
            return true;
        }
        if self.fallback_loader.get().iter().any(|f| f.lookup_glyph_index(c) != 0) {
            return true;
        }
        self.dynamic_fallbacks.lock().unwrap().iter().any(|f: &Arc<Font>| f.lookup_glyph_index(c) != 0)
    }
}

/// Background-thread function that loads fallback fonts for symbols and missing glyphs.
fn load_fallback_fonts_impl() -> Vec<Arc<Font>> {
    let mut fallback_paths: Vec<std::path::PathBuf> = Vec::new();

    // Resolve via fontconfig first (works on NixOS and all distros)
    let fc_families = [
        ("Symbols Nerd Font Mono", "Regular"),
        ("Symbols Nerd Font", "Regular"),
        ("Noto Sans Symbols2", "Regular"),
        ("DejaVu Sans", "Book"),
        ("Noto Sans", "Regular"),
        ("Noto Emoji", "Regular"),
    ];

    // Run fc-match queries in parallel — each is a subprocess (~10-20ms).
    let resolved: Vec<Option<String>> = thread::scope(|s| {
        let handles: Vec<_> = fc_families
            .iter()
            .map(|(family, style)| s.spawn(move || FontCache::fc_match(family, style)))
            .collect();
        handles.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    for path in resolved.into_iter().flatten() {
        fallback_paths.push(std::path::PathBuf::from(path));
    }

    // Add user font directories
    if let Some(home) = dirs::home_dir() {
        let user_fonts = home.join(".local/share/fonts");
        fallback_paths.push(user_fonts.join("NerdFontsSymbols/SymbolsNerdFontMono-Regular.ttf"));
        fallback_paths.push(user_fonts.join("NerdFontsSymbols/SymbolsNerdFont-Regular.ttf"));
    }

    // Hardcoded system paths as final fallback
    let system_paths = [
        "/usr/share/fonts/TTF/SymbolsNerdFont-Regular.ttf",
        "/usr/share/fonts/TTF/SymbolsNerdFontMono-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/google-noto-vf/NotoSansSymbols[wght].ttf",
        "/usr/share/fonts/gdouros-symbola/Symbola.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
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
