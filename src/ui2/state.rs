//! The new shell's own state (`docs/redesign/04-architecture.md` *State*): which panel has the
//! focus, which is zoomed, the preset, the density, the toasts. Pure; the sources never look at it.
//! The fields of later phases (cursors, replay, palette, explain, float, wizard, …) join it then.

use super::layout::{Density, PanelId, Preset};
use std::time::{Duration, SystemTime};

/// A message in the bottom-right corner for a few seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: SystemTime,
}

impl Toast {
    /// 5 s, 10 s for an error (`02-panels.md` *Toasts*).
    pub fn lasts(&self) -> Duration {
        Duration::from_secs(if self.error { 10 } else { 5 })
    }
}

/// At most this many toasts at once; the oldest goes first.
pub const TOASTS: usize = 3;

/// The palette (`:`) or the keys box (`?`), while open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Palette {
    pub query: String,
    /// The row chosen among the matches.
    pub sel: usize,
    /// Which channel: 0 is the actions (`overlay::palette::CHANNELS`).
    pub channel: usize,
    /// Opened with `?`: the keys of the scope only.
    pub help: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellState {
    /// The new shell is the one on screen (`UI=2`); the old one otherwise.
    pub on: bool,
    /// What the top bar names after `cobserve ·`: the cluster, or `fake`.
    pub profile: String,
    pub focus: PanelId,
    pub zoom: Option<PanelId>,
    pub preset: Preset,
    pub density: Density,
    pub toasts: Vec<Toast>,
    /// The chosen node, by name: the NODE panel and the explain popover follow it. `None` until
    /// one is chosen, which reads as the first in the sort order.
    pub chosen: Option<String>,
    pub palette: Option<Palette>,
}

impl Default for ShellState {
    /// The focus starts on NODES (`01-screen.md` *Focus and zoom*).
    fn default() -> ShellState {
        ShellState {
            on: false,
            profile: "prod".into(),
            focus: PanelId::Nodes,
            zoom: None,
            preset: Preset::All,
            density: Density::Comfortable,
            toasts: Vec::new(),
            chosen: None,
            palette: None,
        }
    }
}

impl ShellState {
    /// `1`–`8`: focus a panel on the grid; while zoomed, switch the zoom to it.
    pub fn focus_on(&mut self, panel: PanelId) {
        self.focus = panel;
        if self.zoom.is_some() {
            self.zoom = Some(panel);
        }
    }

    /// `tab` and `shift+tab`.
    pub fn step(&mut self, forward: bool) {
        let next = if forward { self.focus.next() } else { self.focus.prev() };
        self.focus_on(next);
    }

    /// `⏎` on the grid, `f` either way: the focused panel full-screen, or back to the grid.
    pub fn toggle_zoom(&mut self) {
        self.zoom = match self.zoom {
            Some(_) => None,
            None => Some(self.focus),
        };
    }

    /// `esc`: close the top-most thing — here, the zoom. Never quits; says whether it closed
    /// something.
    pub fn back(&mut self) -> bool {
        if self.palette.take().is_some() {
            return true;
        }
        self.zoom.take().is_some()
    }

    pub fn cycle_preset(&mut self) {
        self.preset = self.preset.next();
    }

    pub fn cycle_density(&mut self) {
        self.density = self.density.next();
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool, at: SystemTime) {
        self.toasts.push(Toast { text: text.into(), error, at });
        let over = self.toasts.len().saturating_sub(TOASTS);
        self.toasts.drain(..over);
    }

    /// Drop the toasts whose time is up.
    pub fn expire(&mut self, now: SystemTime) {
        self.toasts.retain(|t| now.duration_since(t.at).map_or(true, |age| age < t.lasts()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_zoom_and_back() {
        let mut s = ShellState::default();
        assert_eq!(s.focus, PanelId::Nodes);
        s.toggle_zoom();
        assert_eq!(s.zoom, Some(PanelId::Nodes));
        s.focus_on(PanelId::Jira);
        assert_eq!(s.zoom, Some(PanelId::Jira), "a digit while zoomed switches the zoom");
        assert!(s.back());
        assert!(!s.back(), "nothing left to close: esc never quits");
        assert_eq!(s.focus, PanelId::Jira, "back on the grid with the focus on that panel");
        s.step(false);
        assert_eq!(s.focus, PanelId::Airflow);
        s.cycle_preset();
        s.cycle_density();
        assert_eq!((s.preset, s.density), (Preset::Fleet, Density::Compact));
    }

    #[test]
    fn three_toasts_at_most_and_each_for_its_time() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let mut s = ShellState::default();
        for i in 0..5 {
            s.toast(format!("t{i}"), i == 4, t0);
        }
        assert_eq!(s.toasts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["t2", "t3", "t4"]);
        s.expire(t0 + Duration::from_secs(6));
        assert_eq!(s.toasts.len(), 1, "the error stays 10 s");
        s.expire(t0 + Duration::from_secs(11));
        assert!(s.toasts.is_empty());
    }
}
