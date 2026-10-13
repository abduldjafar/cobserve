//! The replay stub (`01-screen.md` *Replay*, `screens/120x36-dash-replay.txt`): `[` a minute back,
//! `]` a minute forward, `t` back to live, over the history the app keeps (four minutes; the hour
//! comes with feature A1). While replaying, the clock says so in amber, the graphs light the
//! minute's column, and the numbers history keeps — the fleet's memory and CPU, each node's — are
//! the minute's. Polling goes on underneath, so `t` is instant.

use crate::app::App;
use crate::history::{Series, WINDOW_S};

/// The furthest back the history reaches, in whole minutes.
pub fn reach(app: &App) -> u8 {
    let span = app.history.fleet_mem_pct.span_s().min(WINDOW_S);
    (span / 60.0).floor() as u8
}

/// The moment replayed, in history's seconds; `None` when live.
pub fn moment(app: &App) -> Option<f64> {
    (app.shell.replay > 0).then(|| app.history.now() - f64::from(app.shell.replay) * 60.0)
}

/// What `series` held in the replayed minute: the worst of it, as the graphs show a slice.
pub fn at(app: &App, series: &Series) -> Option<f64> {
    let t = moment(app)?;
    series.buckets(t, 60.0, 1).first().copied().flatten()
}

/// `value` live, or the replayed minute's from `series`.
pub fn value(app: &App, live: Option<f64>, series: Option<&Series>) -> Option<f64> {
    match (moment(app), series) {
        (Some(_), Some(s)) => at(app, s),
        _ => live,
    }
}

pub fn back(app: &mut App) {
    let reach = reach(app);
    if reach == 0 {
        let at = app.clock;
        app.shell.toast("nothing to replay yet: the history fills a minute at a time", false, at);
        return;
    }
    app.shell.replay = (app.shell.replay + 1).min(reach);
}

pub fn forward(app: &mut App) {
    app.shell.replay = app.shell.replay.saturating_sub(1);
}

pub fn live(app: &mut App) {
    app.shell.replay = 0;
}

/// The column of a `width`-wide graph over the history window that the replayed minute falls in.
pub fn column(app: &App, width: u16) -> Option<u16> {
    let back = f64::from(app.shell.replay);
    if back == 0.0 || width == 0 {
        return None;
    }
    let window_min = WINDOW_S / 60.0;
    let from_right = (back * f64::from(width) / window_min).round() as u16;
    width.checked_sub(1 + from_right.min(width - 1))
}
