//! The new shell, mission control (`docs/redesign/`). Phase 0 holds its contracts only — the grid
//! (`layout.rs`) and the shell's state (`state.rs`); nothing draws yet. `UI=2` and `draw` arrive in
//! phase 1, and the old shell (`src/ui/`) goes in phase 6.

// Phase 0: the contracts exist before the shell that uses them; phase 1 draws with them.
#![allow(dead_code)]

pub mod layout;
pub mod state;

pub use layout::PanelId;
pub use state::ShellState;
