//! Colours: one palette, rendered at whatever depth the terminal can show.
//!
//! - 24-bit when the terminal says so (`COLORTERM=truecolor` / `24bit`),
//! - the xterm 256-colour cube when `TERM` mentions 256 colours — the same palette, each
//!   colour snapped to its nearest cube or grey entry,
//! - the 16 ANSI colours on anything older, with no painted background so a light or a
//!   transparent terminal still reads,
//! - no colour at all under `NO_COLOR` (https://no-color.org) or `THEME=mono`: severity is
//!   then carried by bold and by the glyphs, which every screen in this app prints anyway.
//!
//! `THEME=light` swaps the palette for light terminals. Everything else in the UI asks this
//! module for a *role* (`muted`, `crit`, `surface`), never for a colour, so the choice is made
//! in exactly one place.

use crate::severity::Severity;
use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
}

/// The palette as RGB. Converted per depth by `Theme::new`.
#[derive(Debug, Clone, Copy)]
struct Palette {
    bg: (u8, u8, u8),
    panel: (u8, u8, u8),
    raised: (u8, u8, u8),
    selection: (u8, u8, u8),
    border: (u8, u8, u8),
    rule: (u8, u8, u8),
    fg: (u8, u8, u8),
    text2: (u8, u8, u8),
    muted: (u8, u8, u8),
    faint: (u8, u8, u8),
    accent: (u8, u8, u8),
    person: (u8, u8, u8),
    claude: (u8, u8, u8),
    ok: (u8, u8, u8),
    warn: (u8, u8, u8),
    crit: (u8, u8, u8),
    info: (u8, u8, u8),
    bar: (u8, u8, u8),
    server_bar: (u8, u8, u8),
    keycap: (u8, u8, u8),
    tint_ok: (u8, u8, u8),
    tint_warn: (u8, u8, u8),
    tint_crit: (u8, u8, u8),
    tint_claude: (u8, u8, u8),
}

const DARK: Palette = Palette {
    bg: (15, 18, 25),
    panel: (24, 29, 39),
    raised: (33, 39, 53),
    selection: (40, 60, 98),
    border: (70, 82, 104),
    rule: (50, 59, 76),
    fg: (222, 228, 237),
    text2: (172, 182, 197),
    muted: (122, 133, 150),
    faint: (82, 91, 107),
    accent: (110, 182, 255),
    person: (201, 167, 255),
    claude: (217, 119, 87),
    ok: (88, 199, 128),
    warn: (236, 183, 64),
    crit: (250, 108, 98),
    info: (78, 205, 196),
    bar: (82, 146, 222),
    server_bar: (96, 106, 124),
    keycap: (46, 55, 72),
    tint_ok: (22, 49, 37),
    tint_warn: (60, 47, 20),
    tint_crit: (66, 29, 31),
    tint_claude: (61, 38, 31),
};

const LIGHT: Palette = Palette {
    bg: (250, 251, 253),
    panel: (238, 241, 246),
    raised: (226, 231, 239),
    selection: (204, 222, 252),
    border: (200, 207, 218),
    rule: (222, 227, 234),
    fg: (30, 35, 42),
    text2: (66, 74, 86),
    muted: (104, 113, 125),
    faint: (152, 160, 171),
    accent: (9, 105, 218),
    person: (128, 76, 222),
    claude: (190, 88, 52),
    ok: (26, 127, 55),
    warn: (166, 110, 0),
    crit: (207, 34, 46),
    info: (14, 125, 125),
    bar: (60, 118, 204),
    server_bar: (158, 166, 178),
    keycap: (222, 227, 236),
    tint_ok: (218, 241, 226),
    tint_warn: (251, 238, 204),
    tint_crit: (251, 223, 221),
    tint_claude: (250, 226, 214),
};

/// Every colour role the UI uses.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub depth: Depth,
    pub bg: Color,
    pub panel: Color,
    /// A surface on the surface: the session on screen, a tile under the cursor.
    pub raised: Color,
    pub selection: Color,
    pub border: Color,
    pub rule: Color,
    pub fg: Color,
    pub text2: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub person: Color,
    /// Claude Code's own orange, for its sessions.
    pub claude: Color,
    pub ok: Color,
    pub warn: Color,
    pub crit: Color,
    pub info: Color,
    pub bar: Color,
    pub server_bar: Color,
    pub keycap: Color,
    /// Backgrounds a severity colours: a reminder, a status.
    pub tint_ok: Color,
    pub tint_warn: Color,
    pub tint_crit: Color,
    /// Under what a helper just wrote into a query.
    pub tint_claude: Color,
}

impl Theme {
    pub fn new(depth: Depth, variant: Variant) -> Self {
        let p = match variant {
            Variant::Dark => DARK,
            Variant::Light => LIGHT,
        };
        match depth {
            Depth::TrueColor | Depth::Ansi256 => {
                let c = |rgb: (u8, u8, u8)| match depth {
                    Depth::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
                    _ => Color::Indexed(nearest_256(rgb)),
                };
                Theme {
                    depth,
                    bg: c(p.bg),
                    panel: c(p.panel),
                    raised: c(p.raised),
                    selection: c(p.selection),
                    border: c(p.border),
                    rule: c(p.rule),
                    fg: c(p.fg),
                    text2: c(p.text2),
                    muted: c(p.muted),
                    faint: c(p.faint),
                    accent: c(p.accent),
                    person: c(p.person),
                    claude: c(p.claude),
                    ok: c(p.ok),
                    warn: c(p.warn),
                    crit: c(p.crit),
                    info: c(p.info),
                    bar: c(p.bar),
                    server_bar: c(p.server_bar),
                    keycap: c(p.keycap),
                    tint_ok: c(p.tint_ok),
                    tint_warn: c(p.tint_warn),
                    tint_crit: c(p.tint_crit),
                    tint_claude: c(p.tint_claude),
                }
            }
            // 16 colours: the terminal's own background stays, so nothing here paints one.
            Depth::Ansi16 => Theme {
                depth,
                bg: Color::Reset,
                panel: Color::Reset,
                raised: Color::Reset,
                selection: Color::Reset,
                border: Color::DarkGray,
                rule: Color::DarkGray,
                fg: Color::Reset,
                text2: Color::Reset,
                muted: Color::DarkGray,
                faint: Color::DarkGray,
                accent: Color::LightBlue,
                person: Color::LightMagenta,
                claude: Color::LightRed,
                ok: Color::Green,
                warn: Color::Yellow,
                crit: Color::Red,
                info: Color::Cyan,
                bar: Color::Blue,
                server_bar: Color::DarkGray,
                keycap: Color::Reset,
                tint_ok: Color::Reset,
                tint_warn: Color::Reset,
                tint_crit: Color::Reset,
                tint_claude: Color::Reset,
            },
            Depth::Mono => Theme {
                depth,
                bg: Color::Reset,
                panel: Color::Reset,
                raised: Color::Reset,
                selection: Color::Reset,
                border: Color::Reset,
                rule: Color::Reset,
                fg: Color::Reset,
                text2: Color::Reset,
                muted: Color::Reset,
                faint: Color::Reset,
                accent: Color::Reset,
                person: Color::Reset,
                claude: Color::Reset,
                ok: Color::Reset,
                warn: Color::Reset,
                crit: Color::Reset,
                info: Color::Reset,
                bar: Color::Reset,
                server_bar: Color::Reset,
                keycap: Color::Reset,
                tint_ok: Color::Reset,
                tint_warn: Color::Reset,
                tint_crit: Color::Reset,
                tint_claude: Color::Reset,
            },
        }
    }

    /// Whether backgrounds are painted at all: without them the chrome has no surface, and
    /// lines take its place.
    pub fn paints_background(&self) -> bool {
        matches!(self.depth, Depth::TrueColor | Depth::Ansi256)
    }

    pub fn base(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.fg)
    }

    pub fn strong(&self) -> Style {
        Style::default().fg(self.fg).add_modifier(Modifier::BOLD)
    }

    pub fn text2(&self) -> Style {
        Style::default().fg(self.text2)
    }

    pub fn muted(&self) -> Style {
        if self.depth == Depth::Mono {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(self.muted)
        }
    }

    pub fn faint(&self) -> Style {
        if self.depth == Depth::Mono {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(self.faint)
        }
    }

    pub fn accent(&self) -> Style {
        Style::default().fg(self.accent)
    }

    pub fn person(&self) -> Style {
        Style::default().fg(self.person)
    }

    /// Claude Code's mark on its sessions.
    pub fn claude(&self) -> Style {
        Style::default().fg(self.claude).add_modifier(Modifier::BOLD)
    }

    pub fn border(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn rule(&self) -> Style {
        Style::default().fg(self.rule)
    }

    /// Section titles on a rule: `─ INSIGHTS ─`.
    pub fn section(&self) -> Style {
        self.muted().add_modifier(Modifier::BOLD)
    }

    /// The foreground a severity is drawn in. `None` is the plain text colour.
    pub fn sev_fg(&self, sev: Severity) -> Color {
        match sev {
            Severity::None => self.fg,
            Severity::Ok => self.ok,
            Severity::Info => self.info,
            Severity::Warn => self.warn,
            Severity::Crit => self.crit,
        }
    }

    pub fn sev(&self, sev: Severity) -> Style {
        let style = Style::default().fg(self.sev_fg(sev));
        match (self.depth, sev) {
            // Without colour the only way to shout is bold.
            (Depth::Mono, Severity::Crit) | (Depth::Mono, Severity::Warn) => {
                style.add_modifier(Modifier::BOLD)
            }
            _ => style,
        }
    }

    /// The fill of a bar or a sparkline: calm when nothing is wrong, the severity otherwise.
    pub fn bar_fill(&self, sev: Severity) -> Color {
        match sev {
            Severity::None | Severity::Ok | Severity::Info => self.bar,
            other => self.sev_fg(other),
        }
    }

    /// The active view tab.
    pub fn tab_active(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default()
                .fg(self.bg)
                .bg(self.accent)
                .add_modifier(Modifier::BOLD),
            _ => Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        }
    }

    /// A key in the footer: `⏎` on a small raised cap.
    pub fn keycap(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default()
                .fg(self.accent)
                .bg(self.keycap)
                .add_modifier(Modifier::BOLD),
            _ => Style::default().add_modifier(Modifier::BOLD),
        }
    }

    /// The selected row. Painted when the terminal can show a background, reversed when not.
    /// A block of code under its row: on the panel colour where the terminal has one.
    pub fn code(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.panel),
            _ => Style::default(),
        }
    }

    /// The chrome's surface — the shelf at the top, the list beside the sessions, the footer —
    /// above the well the work is drawn in. Nothing where the terminal paints no background.
    pub fn surface(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.panel),
            _ => Style::default(),
        }
    }

    /// A table's header row: the chrome's surface across the well, edge to edge, so the columns'
    /// names read as a band above the rows. Underlined where backgrounds are not painted.
    pub fn table_head(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.panel),
            _ => Style::default().add_modifier(Modifier::UNDERLINED),
        }
    }

    /// A surface on the surface: the session on screen.
    pub fn raised(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.raised),
            _ => Style::default().add_modifier(Modifier::BOLD),
        }
    }

    /// A background in a severity's colour, for what has to be seen — a prayer's reminder, a
    /// status — with text in that colour on it. Reversed where backgrounds are not painted.
    pub fn tint(&self, sev: Severity) -> Style {
        let bg = match sev {
            Severity::Crit => self.tint_crit,
            Severity::Warn => self.tint_warn,
            _ => self.tint_ok,
        };
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().fg(self.sev_fg(sev)).bg(bg),
            Depth::Ansi16 => Style::default().fg(self.sev_fg(sev)).add_modifier(Modifier::REVERSED),
            Depth::Mono => Style::default().add_modifier(Modifier::REVERSED),
        }
    }

    /// The row under the cursor: a band of the selection's colour, its text in bold, so it
    /// reads at a glance among rows that are all alike.
    pub fn selected(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.selection).add_modifier(Modifier::BOLD),
            _ => Style::default().add_modifier(Modifier::REVERSED),
        }
    }

    /// A list over what is under it — a query session's suggestions, its servers — a step
    /// above the raised field it opens from; and the row chosen in it.
    pub fn popup(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.keycap),
            _ => Style::default(),
        }
    }

    /// What a helper wrote in place of a part of the text, until it is changed or run.
    /// Underlined where backgrounds are not painted.
    pub fn written(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.tint_claude),
            _ => Style::default().add_modifier(Modifier::UNDERLINED),
        }
    }

    /// Text selected in a field.
    pub fn marked(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.border),
            _ => Style::default().add_modifier(Modifier::REVERSED),
        }
    }

    pub fn popup_chosen(&self) -> Style {
        match self.depth {
            Depth::TrueColor | Depth::Ansi256 => Style::default().bg(self.border),
            _ => Style::default().add_modifier(Modifier::REVERSED),
        }
    }
}

/// xterm's 256-colour palette: the 6×6×6 cube from 16 and the 24 greys from 232. Picks
/// whichever of the two is nearer, so a dark grey does not turn blue.
pub fn nearest_256((r, g, b): (u8, u8, u8)) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| -> usize {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, l)| (i32::from(**l) - i32::from(v)).abs())
            .map(|(i, _)| i)
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let cube_index = 16 + 36 * ri + 6 * gi + bi;

    let avg = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
    let grey_step = ((avg.saturating_sub(8)) / 10).min(23);
    let grey_value = (8 + grey_step * 10) as u8;
    let grey_index = 232 + grey_step as usize;

    let dist = |(a, b2, c): (u8, u8, u8)| -> i32 {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a, r) + d(b2, g) + d(c, b)
    };
    if dist((grey_value, grey_value, grey_value)) < dist(cube) {
        grey_index as u8
    } else {
        cube_index as u8
    }
}

/// The depth and variant the environment asks for. Pure, so it can be tested without
/// touching the process environment.
pub fn choose(
    no_color: Option<&str>,
    theme: Option<&str>,
    colorterm: Option<&str>,
    term: Option<&str>,
) -> (Depth, Variant) {
    let theme = theme.map(|t| t.trim().to_ascii_lowercase());
    let variant = match theme.as_deref() {
        Some("light") => Variant::Light,
        _ => Variant::Dark,
    };
    if no_color.is_some_and(|v| !v.is_empty()) || theme.as_deref() == Some("mono") {
        return (Depth::Mono, variant);
    }
    let colorterm = colorterm.unwrap_or_default().to_ascii_lowercase();
    let term = term.unwrap_or_default().to_ascii_lowercase();
    let depth = if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        Depth::TrueColor
    } else if term == "dumb" {
        Depth::Mono
    } else if term.contains("256") || term.contains("kitty") || term.contains("wezterm") {
        Depth::Ansi256
    } else if term.is_empty() {
        // No TERM at all is a test harness or a pipe; the cube is the safe middle.
        Depth::Ansi256
    } else {
        Depth::Ansi16
    };
    (depth, variant)
}

fn from_env() -> Theme {
    let get = |name: &str| std::env::var(name).ok();
    let (depth, variant) = choose(
        get("NO_COLOR").as_deref(),
        get("THEME").as_deref(),
        get("COLORTERM").as_deref(),
        get("TERM").as_deref(),
    );
    Theme::new(depth, variant)
}

/// The theme for this process, read from the environment once.
pub fn current() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(from_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_wins_over_everything() {
        let (depth, _) = choose(Some("1"), Some("dark"), Some("truecolor"), Some("xterm-256color"));
        assert_eq!(depth, Depth::Mono);
        // An empty NO_COLOR is not a request (the convention says "present and not empty").
        let (depth, _) = choose(Some(""), None, Some("truecolor"), None);
        assert_eq!(depth, Depth::TrueColor);
    }

    #[test]
    fn depth_follows_the_terminal() {
        assert_eq!(choose(None, None, Some("24bit"), None).0, Depth::TrueColor);
        assert_eq!(choose(None, None, None, Some("xterm-256color")).0, Depth::Ansi256);
        assert_eq!(choose(None, None, None, Some("xterm")).0, Depth::Ansi16);
        assert_eq!(choose(None, None, None, Some("dumb")).0, Depth::Mono);
        assert_eq!(choose(None, Some("mono"), Some("truecolor"), None).0, Depth::Mono);
    }

    #[test]
    fn the_light_variant_is_asked_for_by_name() {
        assert_eq!(choose(None, Some("LIGHT"), None, None).1, Variant::Light);
        assert_eq!(choose(None, None, None, None).1, Variant::Dark);
    }

    #[test]
    fn the_256_mapping_keeps_greys_grey_and_colours_coloured() {
        // Pure black and white land on the cube corners or the grey ramp ends.
        assert!(matches!(nearest_256((0, 0, 0)), 16 | 232));
        assert!(matches!(nearest_256((255, 255, 255)), 231 | 255));
        // The dark background is a grey, not a navy cube entry.
        let bg = nearest_256(DARK.bg);
        assert!((232..=255).contains(&bg), "{bg}");
        // A saturated red stays in the cube.
        let red = nearest_256((250, 108, 98));
        assert!((16..=231).contains(&red), "{red}");
    }

    #[test]
    fn sixteen_colours_paint_no_background() {
        let theme = Theme::new(Depth::Ansi16, Variant::Dark);
        assert!(!theme.paints_background());
        assert_eq!(theme.bg, Color::Reset);
        assert!(Theme::new(Depth::TrueColor, Variant::Dark).paints_background());
    }
}
