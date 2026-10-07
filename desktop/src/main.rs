//! cobserve in a window of its own: the app a Dock icon opens.
//!
//! A window (`tao`) holding a web view (`wry`) holding a terminal (xterm.js, vendored in
//! `assets/vendor`), and in it `cobserve` itself, in a pseudo-terminal (`portable-pty`) — the same
//! program as in any terminal, keys, mouse, colours and its sessions included. Nothing of the
//! monitor is drawn here; this is only the terminal it runs in.
//!
//! Opened from the Dock, an app has none of a terminal's environment. So cobserve runs through
//! your login shell (`$SHELL -l -c`: `.zprofile` and the like, for the `PATH`) — not an
//! interactive one, whose `.zshrc` may wait on a terminal that is not there — with what
//! `~/.config/cobserve/env` sets (`KEY=value` lines, read as written, never by a shell: a password
//! with `&` in it is fine), and `--credential ~/.config/cobserve/credentials.yaml` when that file
//! exists and no other was named. The `cobserve` it runs is `COBSERVE_BIN`, else the one beside
//! this program (inside `Cobserve.app`), else the one on the `PATH`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

/// What reaches the window's loop: from the page, from the program, from the threads that wait on it.
#[derive(Debug)]
enum Message {
    /// The page has its size, in cells: cobserve starts at it.
    Ready(u16, u16),
    Resize(u16, u16),
    /// Keys, a paste, the mouse — what goes to the program.
    Input(Vec<u8>),
    Title(String),
    /// What the program printed.
    Output(Vec<u8>),
    /// Files dropped on the window: their paths go in as a paste, as a terminal's do.
    Dropped(Vec<PathBuf>),
    /// The title strip was pressed (the window moves with the mouse), or double-clicked.
    Drag,
    Zoom,
    /// The program ended: whether it ended well (`q`), and how.
    Exited(bool, String),
}

/// A message of the page's, as `window.ipc.postMessage` sends it.
#[derive(Deserialize)]
struct FromPage {
    t: String,
    #[serde(default)]
    d: serde_json::Value,
    #[serde(default)]
    cols: u16,
    #[serde(default)]
    rows: u16,
}

/// How the terminal looks: your iTerm2 profile's font and colours when there is one, else these
/// — the "Data Engineer" profile's own (Tokyo Night, JetBrains Mono 13).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct Look {
    font: String,
    size: f64,
    background: String,
    foreground: String,
    cursor: String,
    cursor_text: String,
    selection: String,
    selected_text: String,
    /// `bar`, `block` or `underline`.
    cursor_style: String,
    cursor_blink: bool,
    /// The 16 ANSI colours, black to bright white.
    ansi: Vec<String>,
    /// `dom` (the web view's own text, as crisp as the system's) or `webgl`.
    renderer: String,
}

impl Default for Look {
    fn default() -> Look {
        let ansi = [
            "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6", "#414868", "#f7768e", "#9ece6a",
            "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
        ];
        Look {
            font: "JetBrainsMonoNF-Regular".into(),
            size: 13.0,
            background: "#1a1b26".into(),
            foreground: "#c0caf5".into(),
            cursor: "#c0caf5".into(),
            cursor_text: "#1a1b26".into(),
            selection: "#283457".into(),
            selected_text: "#c0caf5".into(),
            cursor_style: "bar".into(),
            cursor_blink: true,
            ansi: ansi.iter().map(|c| c.to_string()).collect(),
            renderer: "dom".into(),
        }
    }
}

/// An iTerm2 colour (`{"Red Component": 0.1, …}`) as `#rrggbb`.
fn iterm_color(value: &serde_json::Value) -> Option<String> {
    let part = |name: &str| value.get(format!("{name} Component")).and_then(serde_json::Value::as_f64);
    let (r, g, b) = (part("Red")?, part("Green")?, part("Blue")?);
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b)))
}

/// The look of an iTerm2 profile: its font (`JetBrainsMonoNF-Regular 13`), colours and cursor;
/// what it does not say stays as it was.
fn look_of(profile: &serde_json::Value, mut look: Look) -> Look {
    if let Some(font) = profile.get("Normal Font").and_then(serde_json::Value::as_str)
        && let Some((name, size)) = font.rsplit_once(' ')
    {
        look.font = name.to_string();
        if let Ok(size) = size.parse::<f64>() {
            look.size = size;
        }
    }
    let color = |key: &str, slot: &mut String| {
        if let Some(c) = profile.get(key).and_then(iterm_color) {
            *slot = c;
        }
    };
    color("Background Color", &mut look.background);
    color("Foreground Color", &mut look.foreground);
    color("Cursor Color", &mut look.cursor);
    color("Cursor Text Color", &mut look.cursor_text);
    color("Selection Color", &mut look.selection);
    color("Selected Text Color", &mut look.selected_text);
    for (i, slot) in look.ansi.iter_mut().enumerate() {
        if let Some(c) = profile.get(format!("Ansi {i} Color")).and_then(iterm_color) {
            *slot = c;
        }
    }
    // iTerm2: 0 underline, 1 vertical bar, 2 box.
    if let Some(kind) = profile.get("Cursor Type").and_then(serde_json::Value::as_i64) {
        look.cursor_style = match kind {
            0 => "underline",
            2 => "block",
            _ => "bar",
        }
        .into();
    }
    if let Some(blink) = profile.get("Blinking Cursor").and_then(serde_json::Value::as_bool) {
        look.cursor_blink = blink;
    }
    look
}

/// The profile named `name` among iTerm2's dynamic profiles, if it is there.
fn iterm_profile(name: &str) -> Option<serde_json::Value> {
    let dir = home().join("Library/Application Support/iTerm2/DynamicProfiles");
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let text = std::fs::read_to_string(entry.path()).ok()?;
        let file: serde_json::Value = serde_json::from_str(&text).ok()?;
        file.get("Profiles")?.as_array()?.iter().find(|p| p.get("Name").and_then(serde_json::Value::as_str) == Some(name)).cloned()
    })
}

/// The look: the iTerm2 profile `COBSERVE_ITERM_PROFILE` names ("Data Engineer" when nothing
/// does), and `COBSERVE_RENDERER`.
fn look(env: &[(String, String)]) -> Look {
    let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).or_else(|| std::env::var(key).ok());
    let name = get("COBSERVE_ITERM_PROFILE").unwrap_or_else(|| "Data Engineer".to_string());
    let mut look = match iterm_profile(&name) {
        Some(profile) => look_of(&profile, Look::default()),
        None => Look::default(),
    };
    if let Some(renderer) = get("COBSERVE_RENDERER").filter(|r| r == "webgl" || r == "dom") {
        look.renderer = renderer;
    }
    look
}

/// The app's own environment file, as pairs.
fn app_env() -> Vec<(String, String)> {
    std::fs::read_to_string(home().join(".config/cobserve/env")).map(|text| env_file(&text)).unwrap_or_default()
}

/// The page: the terminal, xterm.js and its addons put in where they go — one file, nothing
/// fetched — and how it looks.
fn page(look: &Look) -> String {
    include_str!("../assets/index.html")
        .replace("/*LOOK*/", &serde_json::to_string(look).unwrap_or_else(|_| "{}".into()))
        .replace("/*XTERM_CSS*/", include_str!("../assets/vendor/xterm.css"))
        .replace("/*XTERM_JS*/", include_str!("../assets/vendor/xterm.js"))
        .replace("/*FIT_JS*/", include_str!("../assets/vendor/addon-fit.js"))
        .replace("/*UNICODE11_JS*/", include_str!("../assets/vendor/addon-unicode11.js"))
        .replace("/*WEBGL_JS*/", include_str!("../assets/vendor/addon-webgl.js"))
}

/// The `cobserve` to run: `COBSERVE_BIN`, else the one beside this program, else the `PATH`'s.
fn cobserve_bin() -> String {
    if let Ok(bin) = std::env::var("COBSERVE_BIN")
        && !bin.trim().is_empty()
    {
        return bin;
    }
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("cobserve")));
    match beside {
        Some(path) if path.is_file() => path.to_string_lossy().into_owned(),
        _ => "cobserve".to_string(),
    }
}

/// `#1a1b26` as the web view's background before the page paints, so nothing flashes white.
fn rgb(hex: &str) -> (u8, u8, u8, u8) {
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(0);
    (byte(1), byte(3), byte(5), 255)
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// What cobserve is given: this program's own arguments, and the credential file in the usual
/// place when none is named.
fn cobserve_args() -> Vec<String> {
    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn_")).collect();
    let named = args.iter().any(|a| a.starts_with("--credential") || a == "-c");
    let usual = home().join(".config/cobserve/credentials.yaml");
    if !named && usual.is_file() {
        args.push("--credential".into());
        args.push(usual.to_string_lossy().into_owned());
    }
    args
}

/// `KEY=value` lines — `export` before them, quotes round the value, `#` comments and blank
/// lines allowed — as pairs. Nothing is expanded: what is written is what is set.
fn env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            let value = value.trim();
            let unquoted = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            Some((key.to_string(), unquoted.to_string()))
        })
        .collect()
}

/// A path as a shell reads it, the way a terminal types a dropped file: spaces, quotes and the
/// rest of a shell's own characters each behind a backslash.
fn shell_escaped(path: &std::path::Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        if c.is_whitespace() || "\\'\"`$&|;<>()[]{}*?!#~^".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The folders installers put programs in for one user — OpenCode's, Claude Code's, bun's,
/// cargo's — that a terminal finds because `.zshrc` puts them on the `PATH`. The app cannot read
/// `.zshrc` (see `command`), so those that exist go first on the `PATH` it starts with.
fn user_bins(home: &std::path::Path) -> Vec<PathBuf> {
    [".opencode/bin", ".claude/local", ".local/bin", ".bun/bin", ".cargo/bin", ".npm-global/bin", "go/bin", ".deno/bin"]
        .iter()
        .map(|dir| home.join(dir))
        .chain(["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"].iter().map(PathBuf::from))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// cobserve, through the login shell so it has your `PATH`, with the app's own environment.
fn command() -> CommandBuilder {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".to_string());
    let mut command = CommandBuilder::new(&shell);
    // `exec "$0" "$@"`: the shell gives way to cobserve, its arguments passed as they are — after
    // the user's own bin folders go on the `PATH` the profile made, which may have set it anew.
    command.args(["-l", "-c", "[ -n \"$COBSERVE_BINS\" ] && PATH=\"$COBSERVE_BINS:$PATH\"; exec \"$0\" \"$@\""]);
    command.arg(cobserve_bin());
    for arg in cobserve_args() {
        command.arg(arg);
    }
    command.cwd(home());
    if let Ok(bins) = std::env::join_paths(user_bins(&home())) {
        command.env("COBSERVE_BINS", bins);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "cobserve-desktop");
    if std::env::var_os("LANG").is_none() {
        command.env("LANG", "en_US.UTF-8");
    }
    for (key, value) in app_env() {
        command.env(key, value);
    }
    command
}

/// The program in its pseudo-terminal, while it runs.
struct Running {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
}

/// Start cobserve at `cols` × `rows`: what it prints, and its end, come back as messages.
fn start(cols: u16, rows: u16, proxy: &EventLoopProxy<Message>) -> Result<Running, String> {
    let size = PtySize { rows: rows.max(10), cols: cols.max(40), pixel_width: 0, pixel_height: 0 };
    let pair = native_pty_system().openpty(size).map_err(|e| format!("no pseudo-terminal: {e}"))?;
    let mut child = pair.slave.spawn_command(command()).map_err(|e| format!("cobserve would not start: {e}"))?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let output = proxy.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    if output.send_event(Message::Output(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
    });
    let ended = proxy.clone();
    std::thread::spawn(move || {
        let (well, why) = match child.wait() {
            Ok(status) if status.success() => (true, String::new()),
            Ok(status) => (false, format!("cobserve stopped (exit code {}) — what it said is above · ⌘W closes", status.exit_code())),
            Err(e) => (false, format!("cobserve stopped: {e} · ⌘W closes")),
        };
        let _ = ended.send_event(Message::Exited(well, why));
    });
    Ok(Running { master: pair.master, writer })
}

/// The macOS menu: the app's, Edit (so ⌘C, ⌘V and ⌘A reach the terminal) and Window.
#[cfg(target_os = "macos")]
fn menu() -> muda::Menu {
    use muda::{Menu, PredefinedMenuItem, Submenu};
    let menu = Menu::new();
    let app = Submenu::with_items(
        "cobserve",
        true,
        &[
            &PredefinedMenuItem::about(Some("About cobserve"), None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(Some("Quit cobserve")),
        ],
    );
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[&PredefinedMenuItem::copy(None), &PredefinedMenuItem::paste(None), &PredefinedMenuItem::select_all(None)],
    );
    let window = Submenu::with_items(
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::fullscreen(None), &PredefinedMenuItem::close_window(None)],
    );
    if let (Ok(app), Ok(edit), Ok(window)) = (app, edit, window) {
        let _ = menu.append_items(&[&app, &edit, &window]);
    }
    menu.init_for_nsapp();
    menu
}

fn main() {
    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    #[cfg(target_os = "macos")]
    let _menu = menu();
    let look = look(&app_env());
    let builder = WindowBuilder::new();
    // The title bar a part of the window, dark, the screen running up under it as a terminal's does.
    #[cfg(target_os = "macos")]
    let builder = {
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true)
    };
    let window = match builder
        .with_theme(Some(tao::window::Theme::Dark))
        .with_title("cobserve")
        .with_inner_size(LogicalSize::new(1440.0, 900.0))
        .with_min_inner_size(LogicalSize::new(720.0, 420.0))
        .with_position(tao::dpi::LogicalPosition::new(60.0, 60.0))
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(e) => {
            eprintln!("cobserve-desktop: no window: {e}");
            std::process::exit(1);
        }
    };

    let proxy = event_loop.create_proxy();
    let from_page = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_html(page(&look))
        .with_background_color(rgb(&look.background))
        .with_devtools(cfg!(debug_assertions))
        .with_drag_drop_handler({
            let proxy = proxy.clone();
            move |event| match event {
                wry::DragDropEvent::Drop { paths, .. } if !paths.is_empty() => {
                    let _ = proxy.send_event(Message::Dropped(paths));
                    true
                }
                // Nothing dragged over the window may open in it as a page.
                _ => true,
            }
        })
        .with_ipc_handler(move |request| {
            let Ok(message) = serde_json::from_str::<FromPage>(request.body()) else {
                return;
            };
            let event = match message.t.as_str() {
                "ready" => Message::Ready(message.cols, message.rows),
                "resize" => Message::Resize(message.cols, message.rows),
                "in" => Message::Input(message.d.as_str().unwrap_or_default().as_bytes().to_vec()),
                "bin" => Message::Input(
                    message.d.as_array().map(|a| a.iter().filter_map(|b| b.as_u64()).map(|b| b as u8).collect()).unwrap_or_default(),
                ),
                "title" => Message::Title(message.d.as_str().unwrap_or_default().to_string()),
                "drag" => Message::Drag,
                "zoom" => Message::Zoom,
                "log" => {
                    eprintln!("cobserve-desktop: page: {}", message.d.as_str().unwrap_or_default());
                    return;
                }
                _ => return,
            };
            let _ = from_page.send_event(event);
        })
        .build(&window);
    let webview = match webview {
        Ok(webview) => webview,
        Err(e) => {
            eprintln!("cobserve-desktop: no web view: {e}");
            std::process::exit(1);
        }
    };

    let mut running: Option<Running> = None;
    let engine = base64::engine::general_purpose::STANDARD;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                // The pseudo-terminal closes with it: cobserve takes that as a closed terminal and
                // keeps its sessions for the next start.
                running = None;
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(message) => match message {
                Message::Ready(cols, rows) if running.is_none() => match start(cols, rows, &proxy) {
                    Ok(started) => running = Some(started),
                    Err(why) => {
                        let _ = webview.evaluate_script(&format!("window.ended({})", serde_json::Value::String(why)));
                    }
                },
                Message::Ready(..) => {}
                Message::Resize(cols, rows) => {
                    if let Some(run) = &running {
                        let _ = run.master.resize(PtySize { rows: rows.max(10), cols: cols.max(40), pixel_width: 0, pixel_height: 0 });
                    }
                }
                Message::Input(bytes) => {
                    if let Some(run) = running.as_mut() {
                        let _ = run.writer.write_all(&bytes);
                        let _ = run.writer.flush();
                    }
                }
                Message::Title(title) => {
                    let title = title.trim();
                    window.set_title(if title.is_empty() { "cobserve" } else { title });
                }
                Message::Dropped(paths) => {
                    let text = paths.iter().map(|p| shell_escaped(p)).collect::<Vec<_>>().join(" ") + " ";
                    let _ = webview.evaluate_script(&format!("window.dropped({})", serde_json::Value::String(text)));
                }
                Message::Drag => {
                    let _ = window.drag_window();
                }
                Message::Zoom => window.set_maximized(!window.is_maximized()),
                Message::Output(bytes) => {
                    let _ = webview.evaluate_script(&format!("window.out('{}')", engine.encode(&bytes)));
                }
                // `q`: the window goes with it. A failure stays, so what it said can be read — a
                // configuration missing, a `cobserve` not on the PATH.
                Message::Exited(true, _) => {
                    running = None;
                    *control_flow = ControlFlow::Exit;
                }
                Message::Exited(false, why) => {
                    running = None;
                    let _ = webview.evaluate_script(&format!("window.ended({})", serde_json::Value::String(why)));
                }
            },
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_iterm_profile_gives_its_font_colours_and_cursor() {
        let profile: serde_json::Value = serde_json::from_str(
            r#"{ "Name": "Data Engineer", "Normal Font": "JetBrainsMonoNF-Regular 14",
                 "Background Color": { "Red Component": 0.1019, "Green Component": 0.1058, "Blue Component": 0.149 },
                 "Ansi 1 Color": { "Red Component": 0.968, "Green Component": 0.462, "Blue Component": 0.556 },
                 "Cursor Type": 2, "Blinking Cursor": false }"#,
        )
        .unwrap();
        let look = look_of(&profile, Look::default());
        assert_eq!((look.font.as_str(), look.size), ("JetBrainsMonoNF-Regular", 14.0));
        assert_eq!((look.background.as_str(), look.ansi[1].as_str()), ("#1a1b26", "#f7768e"));
        assert_eq!((look.cursor_style.as_str(), look.cursor_blink), ("block", false));
        assert_eq!(look.ansi[2], Look::default().ansi[2], "what it does not say stays");
        assert_eq!(rgb("#1a1b26"), (26, 27, 38, 255));
    }

    #[test]
    fn the_page_carries_the_terminal_and_fetches_nothing() {
        let page = page(&Look::default());
        assert!(page.contains("\"font\":\"JetBrainsMonoNF-Regular\""), "the look put in");
        assert!(!page.contains("/*XTERM_JS*/") && !page.contains("/*WEBGL_JS*/"), "every part put in");
        assert!(page.contains("FitAddon") && page.contains("Unicode11Addon") && page.contains("WebglAddon"));
        assert!(!page.contains("src=\"http"), "nothing from the network");
    }

    #[test]
    fn the_env_file_is_taken_as_written() {
        let pairs = env_file("# cobserve\nexport JIRA_URL=https://jira.example.net\nJIRA_TOKEN=\"abc=def\"\nAIRFLOW_PASSWORD=p&ss$word\n\nnot a line\nBAD KEY=1\nPRAYER_CITY='Bandung'\n");
        assert_eq!(
            pairs,
            [
                ("JIRA_URL".to_string(), "https://jira.example.net".to_string()),
                ("JIRA_TOKEN".to_string(), "abc=def".to_string()),
                ("AIRFLOW_PASSWORD".to_string(), "p&ss$word".to_string()),
                ("PRAYER_CITY".to_string(), "Bandung".to_string()),
            ]
        );
    }

    #[test]
    fn a_dropped_path_is_typed_as_a_shell_reads_it() {
        assert_eq!(shell_escaped(std::path::Path::new("/Users/me/Screen Shot (2).png")), "/Users/me/Screen\\ Shot\\ \\(2\\).png");
        assert_eq!(shell_escaped(std::path::Path::new("/tmp/plain.txt")), "/tmp/plain.txt");
    }

    #[test]
    fn the_user_s_own_bins_are_found_when_they_exist() {
        let home = std::env::temp_dir().join(format!("cobserve-bins-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".opencode/bin")).unwrap();
        let bins = user_bins(&home);
        assert_eq!(bins.first(), Some(&home.join(".opencode/bin")), "{bins:?}");
        assert!(!bins.contains(&home.join(".bun/bin")), "only those there");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_message_of_the_page_reads() {
        let m: FromPage = serde_json::from_str(r#"{"t":"resize","cols":140,"rows":40}"#).unwrap();
        assert_eq!((m.t.as_str(), m.cols, m.rows), ("resize", 140, 40));
        let m: FromPage = serde_json::from_str(r#"{"t":"bin","d":[27,91,77]}"#).unwrap();
        assert_eq!(m.d.as_array().map(Vec::len), Some(3));
    }
}
