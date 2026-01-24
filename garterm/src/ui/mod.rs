//! UI components for tabs, panes, and splits
//!
//! This module provides the data structures for managing multiple terminals
//! in a tabbed, split-pane interface.

mod manager;
mod pane;
mod split;
mod tab;
mod tabbar;

pub use manager::TabManager;
pub use pane::{Pane, PaneId};
pub use split::{Direction, PaneLayout, SplitDirection, SplitNode};
pub use tab::{Tab, TabId};
pub use tabbar::{TabBar, TabBarRenderData, TabRect, TabRenderInfo, TAB_BAR_HEIGHT};
