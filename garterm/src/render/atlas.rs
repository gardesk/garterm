use super::font::{FontCache, FontStyle};
use std::collections::HashMap;

/// Key for glyph cache lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub c: char,
    pub style: FontStyle,
}

/// Entry in the glyph atlas
#[derive(Debug, Clone, Copy)]
pub struct GlyphEntry {
    /// UV coordinates in atlas (x, y, width, height) in pixels
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Bearing (offset from cursor to top-left of glyph)
    pub bearing_x: i32,
    pub bearing_y: i32,
}

/// Glyph texture atlas using simple row-based packing
pub struct GlyphAtlas {
    /// Atlas texture data (single channel)
    data: Vec<u8>,
    /// Atlas dimensions
    width: u32,
    height: u32,
    /// Current packing position
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    /// Glyph cache
    cache: HashMap<GlyphKey, GlyphEntry>,
    /// Whether atlas needs re-upload to GPU
    dirty: bool,
}

impl GlyphAtlas {
    /// Create a new glyph atlas
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            data: vec![0u8; (width * height) as usize],
            width,
            height,
            cursor_x: 1, // 1 pixel padding
            cursor_y: 1,
            row_height: 0,
            cache: HashMap::new(),
            dirty: false,
        }
    }

    /// Get atlas dimensions
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Get atlas texture data
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Check if atlas needs re-upload
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear dirty flag
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Get or insert a glyph
    pub fn get_or_insert(
        &mut self,
        key: GlyphKey,
        fonts: &FontCache,
    ) -> Option<GlyphEntry> {
        if let Some(&entry) = self.cache.get(&key) {
            return Some(entry);
        }

        // Rasterize the glyph
        let (metrics, bitmap) = fonts.rasterize(key.c, key.style);

        if metrics.width == 0 || metrics.height == 0 {
            // Space or invisible character - return a dummy entry
            let entry = GlyphEntry {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
            };
            self.cache.insert(key, entry);
            return Some(entry);
        }

        let glyph_width = metrics.width as u32;
        let glyph_height = metrics.height as u32;

        // Check if we need to move to next row
        if self.cursor_x + glyph_width + 1 > self.width {
            self.cursor_x = 1;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }

        // Check if atlas is full
        if self.cursor_y + glyph_height + 1 > self.height {
            tracing::warn!("Glyph atlas full, cannot add more glyphs");
            return None;
        }

        // Copy glyph data to atlas
        let atlas_x = self.cursor_x;
        let atlas_y = self.cursor_y;

        for y in 0..glyph_height {
            let src_start = (y * glyph_width) as usize;
            let src_end = src_start + glyph_width as usize;
            let dst_start = ((atlas_y + y) * self.width + atlas_x) as usize;

            self.data[dst_start..dst_start + glyph_width as usize]
                .copy_from_slice(&bitmap[src_start..src_end]);
        }

        // Update packing state
        self.cursor_x += glyph_width + 1;
        self.row_height = self.row_height.max(glyph_height);
        self.dirty = true;

        let entry = GlyphEntry {
            x: atlas_x,
            y: atlas_y,
            width: glyph_width,
            height: glyph_height,
            bearing_x: metrics.xmin,
            bearing_y: metrics.ymin,
        };

        self.cache.insert(key, entry);
        Some(entry)
    }

    /// Clear the atlas (e.g., on font size change)
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.cursor_x = 1;
        self.cursor_y = 1;
        self.row_height = 0;
        self.cache.clear();
        self.dirty = true;
    }
}
