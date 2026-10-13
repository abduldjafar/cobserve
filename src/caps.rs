//! What the terminal can do, asked once before the event loop (`docs/redesign/04-architecture.md`
//! *Capability probe*): its colour depth, whether it speaks the kitty keyboard protocol, whether it
//! draws block elements one cell wide, and which program it is. The new shell picks its graph tier,
//! its painted backgrounds and its link style from these.

#![allow(dead_code)]

/// How graphs are drawn (`docs/redesign/03-glyphs.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphTier {
    /// Two samples a column, as btop does.
    #[default]
    Braille,
    /// `▁▂▃▄▅▆▇█`.
    Block,
    /// `.:-=+*#`.
    Ascii,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    pub depth: crate::theme::Depth,
    /// The kitty keyboard protocol: `ctrl+\` and `⇧⏎` come through as themselves.
    pub keyboard: bool,
    /// Block elements drawn two cells wide (a wide-ambiguous terminal).
    pub wide_blocks: bool,
    pub graph: GraphTier,
    /// `$TERM_PROGRAM`, for the OSC 8 and OSC 9 heuristics.
    pub term_program: String,
}

impl Default for Caps {
    /// What is assumed before the probe, and in the tests: a 24-bit terminal of narrow blocks.
    fn default() -> Caps {
        Caps { depth: crate::theme::Depth::TrueColor, keyboard: false, wide_blocks: false, graph: GraphTier::Braille, term_program: String::new() }
    }
}

/// The graph tier from `GRAPH`, `TERM` and the width check: `GRAPH` wins; a dumb terminal gets
/// ascii; blocks drawn wide rule the block tier out, and braille shares their width class.
pub fn graph_tier(graph: Option<&str>, term: Option<&str>, wide_blocks: bool) -> GraphTier {
    match graph.map(|g| g.trim().to_ascii_lowercase()).as_deref() {
        Some("ascii") => return GraphTier::Ascii,
        Some("block") if !wide_blocks => return GraphTier::Block,
        Some("braille") if !wide_blocks => return GraphTier::Braille,
        _ => {}
    }
    if term == Some("dumb") || wide_blocks {
        return GraphTier::Ascii;
    }
    GraphTier::Braille
}

/// Prints `▂` at the top-left corner, asks where the cursor went (`CSI 6n`) and wipes the line:
/// two cells means the terminal draws block elements wide. Only before anything reads keys, as
/// the answer arrives on standard input; any failure reads as "one cell".
pub fn blocks_draw_wide() -> bool {
    use crossterm::{cursor, execute, style::Print, terminal};
    let mut out = std::io::stdout();
    if execute!(out, cursor::MoveTo(0, 0), Print("▂")).is_err() {
        return false;
    }
    let wide = matches!(cursor::position(), Ok((column, _)) if column >= 2);
    let _ = execute!(out, cursor::MoveTo(0, 0), terminal::Clear(terminal::ClearType::CurrentLine));
    wide
}

/// The probe: `keyboard` is what `main.rs` already learned when it pushed the protocol.
pub fn probe(keyboard: bool) -> Caps {
    let wide_blocks = blocks_draw_wide();
    Caps {
        depth: crate::theme::current().depth,
        keyboard,
        wide_blocks,
        graph: graph_tier(std::env::var("GRAPH").ok().as_deref(), std::env::var("TERM").ok().as_deref(), wide_blocks),
        term_program: std::env::var("TERM_PROGRAM").unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graph_tier_follows_graph_term_and_the_width_check() {
        assert_eq!(graph_tier(None, Some("xterm-256color"), false), GraphTier::Braille, "braille by default");
        assert_eq!(graph_tier(Some("block"), None, false), GraphTier::Block);
        assert_eq!(graph_tier(Some("ASCII"), None, false), GraphTier::Ascii);
        assert_eq!(graph_tier(None, Some("dumb"), false), GraphTier::Ascii);
        assert_eq!(graph_tier(Some("block"), None, true), GraphTier::Ascii, "blocks drawn wide: not blocks, nor braille");
        assert_eq!(graph_tier(Some("nonsense"), None, false), GraphTier::Braille);
    }
}
