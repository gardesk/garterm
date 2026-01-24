//! Search state and UI for terminal search

use crate::terminal::SearchMatch;

/// Search state for a pane
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// Whether search mode is active
    pub active: bool,
    /// Current search query
    pub query: String,
    /// All matches found
    pub matches: Vec<SearchMatch>,
    /// Current match index (which match is highlighted)
    pub current_match: usize,
    /// Case-insensitive search
    pub case_insensitive: bool,
}

impl SearchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start search mode
    pub fn start(&mut self) {
        self.active = true;
        self.query.clear();
        self.matches.clear();
        self.current_match = 0;
    }

    /// Cancel search and return to normal mode
    pub fn cancel(&mut self) {
        self.active = false;
        self.query.clear();
        self.matches.clear();
        self.current_match = 0;
    }

    /// Confirm search and stay in normal mode with highlights
    pub fn confirm(&mut self) {
        self.active = false;
        // Keep matches for highlighting
    }

    /// Clear search highlights
    pub fn clear(&mut self) {
        self.matches.clear();
        self.current_match = 0;
    }

    /// Update matches from search results
    pub fn set_matches(&mut self, matches: Vec<SearchMatch>) {
        self.matches = matches;
        if self.current_match >= self.matches.len() {
            self.current_match = 0;
        }
    }

    /// Navigate to next match
    pub fn next_match(&mut self) {
        if !self.matches.is_empty() {
            self.current_match = (self.current_match + 1) % self.matches.len();
        }
    }

    /// Navigate to previous match
    pub fn prev_match(&mut self) {
        if !self.matches.is_empty() {
            self.current_match = if self.current_match == 0 {
                self.matches.len() - 1
            } else {
                self.current_match - 1
            };
        }
    }

    /// Get the current match
    pub fn current(&self) -> Option<&SearchMatch> {
        self.matches.get(self.current_match)
    }

    /// Check if a position is within any match
    pub fn is_match(&self, row: usize, col: usize) -> bool {
        self.matches.iter().any(|m| {
            m.row == row && col >= m.col_start && col < m.col_end
        })
    }

    /// Check if a position is within the current (highlighted) match
    pub fn is_current_match(&self, row: usize, col: usize) -> bool {
        if let Some(m) = self.current() {
            m.row == row && col >= m.col_start && col < m.col_end
        } else {
            false
        }
    }

    /// Get match count text for display
    pub fn match_count_text(&self) -> String {
        if self.matches.is_empty() {
            if self.query.is_empty() {
                String::new()
            } else {
                "No matches".to_string()
            }
        } else {
            format!("{}/{}", self.current_match + 1, self.matches.len())
        }
    }

    /// Add a character to the query
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
    }

    /// Remove last character from query
    pub fn pop_char(&mut self) {
        self.query.pop();
    }

    /// Toggle case sensitivity
    pub fn toggle_case_sensitive(&mut self) {
        self.case_insensitive = !self.case_insensitive;
    }
}
