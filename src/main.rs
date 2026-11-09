//! discord-rs — a lightweight native Discord client.
//!
//! Wraps the official Discord web app in the operating system's own webview
//! (WebKitGTK on Linux, WebView2 on Windows, WKWebView on macOS) instead of
//! bundling a whole Chromium like the Electron app does.

// Release builds on Windows are GUI apps: no console window behind Discord.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{
    fs,
    path::{Path, PathBuf},
};

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Fullscreen, WindowBuilder},
};
use wry::{NewWindowResponse, PermissionKind, PermissionResponse, WebContext, WebViewBuilder};

const APP_NAME: &str = "discord-rs";
const DEFAULT_SIZE: (f64, f64) = (1280.0, 800.0);
const ZOOM_STEP: f64 = 0.1;

/// Commands sent from the page (keyboard shortcuts) or webview callbacks to
/// the main event loop, which owns the window and webview.
#[derive(Debug)]
enum UserEvent {
    Title(String),
    Reload,
    ToggleDevtools,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleFullscreen,
    Navigate(String),
}

/// Keyboard shortcuts injected into the top-level Discord page. They are
/// forwarded to Rust over the IPC bridge, since the webview itself has no
/// menu bar to hang accelerators on.
const SHORTCUTS_JS: &str = r#"
(() => {
  const send = (cmd) => window.ipc && window.ipc.postMessage(cmd);
  window.addEventListener('keydown', (e) => {
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key;
    let cmd = null;
    if (k === 'F12' || (mod && e.shiftKey && (k === 'I' || k === 'i'))) cmd = 'devtools';
    else if (k === 'F5' || (mod && !e.shiftKey && (k === 'r' || k === 'R'))) cmd = 'reload';
    else if (mod && (k === '=' || k === '+')) cmd = 'zoom_in';
    else if (mod && k === '-') cmd = 'zoom_out';
    else if (mod && k === '0') cmd = 'zoom_reset';
    else if (k === 'F11') cmd = 'fullscreen';
    if (cmd) { e.preventDefault(); e.stopPropagation(); send(cmd); }
  }, true);
})();
"#;

fn main() -> wry::Result<()> {
    let start_url = start_url_from_args();

    let data_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME);
    let _ = fs::create_dir_all(&data_dir);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let (w, h) = load_window_size(&data_dir).unwrap_or(DEFAULT_SIZE);
    let window = WindowBuilder::new()
        .with_title("Discord")
        .with_inner_size(LogicalSize::new(w, h))
        .with_min_inner_size(LogicalSize::new(480.0, 360.0))
        .build(&event_loop)
        .expect("failed to create window");

    // A persistent web context keeps cookies + localStorage on disk, so you
    // stay logged in between launches.
    let mut web_context = WebContext::new(Some(data_dir.join("webdata")));

    let title_proxy = proxy.clone();
    let ipc_proxy = proxy.clone();
    let nav_proxy = proxy.clone();
    let downloads_dir = dirs::download_dir().unwrap_or_else(|| data_dir.join("downloads"));

    let mut builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_url(&start_url)
        .with_devtools(true)
        .with_clipboard(true)
        .with_initialization_script_for_main_only(SHORTCUTS_JS, true)
        .with_document_title_changed_handler(move |title| {
            let _ = title_proxy.send_event(UserEvent::Title(title));
        })
        .with_ipc_handler(move |req| {
            let ev = match req.body().as_str() {
                "reload" => UserEvent::Reload,
                "devtools" => UserEvent::ToggleDevtools,
                "zoom_in" => UserEvent::ZoomIn,
                "zoom_out" => UserEvent::ZoomOut,
                "zoom_reset" => UserEvent::ZoomReset,
                "fullscreen" => UserEvent::ToggleFullscreen,
                _ => return,
            };
            let _ = ipc_proxy.send_event(ev);
        })
        // Links Discord opens in a new tab: keep Discord links (invites,
        // message links) inside the app, send everything else to your
        // regular browser.
        .with_new_window_req_handler(move |url, _features| {
            if is_discord_url(&url) {
                let _ = nav_proxy.send_event(UserEvent::Navigate(url));
            } else if let Err(e) = open::that_detached(&url) {
                eprintln!("[{APP_NAME}] could not open {url}: {e}");
            }
            NewWindowResponse::Deny
        })
        // Voice/video calls, screen share and desktop notifications.
        .with_permission_handler(|kind| match kind {
            PermissionKind::Microphone
            | PermissionKind::Camera
            | PermissionKind::DisplayCapture
            | PermissionKind::Notifications
            | PermissionKind::ClipboardRead
            | PermissionKind::Autoplay => PermissionResponse::Allow,
            _ => PermissionResponse::Deny,
        })
        .with_download_started_handler(move |url, path| {
            let name = download_name(&url, path);
            *path = unique_path(&downloads_dir, &name);
            eprintln!("[{APP_NAME}] downloading to {}", path.display());
            true
        });

    if let Ok(ua) = std::env::var("DISCORD_RS_USER_AGENT") {
        builder = builder.with_user_agent(ua);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let webview = builder.build(&window)?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("no GTK vbox"))?
    };

    let mut zoom = 1.0_f64;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        // The web context must outlive the webview; keep it owned here.
        let _ = &web_context;

        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if !window.is_maximized() && window.fullscreen().is_none() {
                    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
                    save_window_size(&data_dir, size.width, size.height);
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(ev) => match ev {
                UserEvent::Title(t) => {
                    let t = t.trim();
                    window.set_title(if t.is_empty() { "Discord" } else { t });
                }
                UserEvent::Reload => {
                    let _ = webview.reload();
                }
                UserEvent::ToggleDevtools => {
                    if webview.is_devtools_open() {
                        webview.close_devtools();
                    } else {
                        webview.open_devtools();
                    }
                }
                UserEvent::ZoomIn | UserEvent::ZoomOut | UserEvent::ZoomReset => {
                    zoom = match ev {
                        UserEvent::ZoomIn => (zoom + ZOOM_STEP).min(3.0),
                        UserEvent::ZoomOut => (zoom - ZOOM_STEP).max(0.3),
                        _ => 1.0,
                    };
                    let _ = webview.zoom(zoom);
                }
                UserEvent::ToggleFullscreen => {
                    let fs = window.fullscreen().is_none().then_some(Fullscreen::Borderless(None));
                    window.set_fullscreen(fs);
                }
                UserEvent::Navigate(url) => {
                    let _ = webview.load_url(&url);
                }
            },
            _ => {}
        }
    });
}

/// `discord-rs` opens the stable client. `--ptb` / `--canary` pick the test
/// builds, and any discord.com URL (e.g. a channel link) opens directly.
fn start_url_from_args() -> String {
    let mut url = "https://discord.com/app".to_string();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--ptb" => url = "https://ptb.discord.com/app".into(),
            "--canary" => url = "https://canary.discord.com/app".into(),
            "-h" | "--help" => {
                println!(
                    "{APP_NAME} — lightweight Discord client\n\n\
                     Usage: {APP_NAME} [--ptb | --canary | <discord.com URL>]\n\n\
                     Shortcuts: Ctrl+R reload · Ctrl+= / Ctrl+- / Ctrl+0 zoom · \
                     F11 fullscreen · F12 or Ctrl+Shift+I devtools"
                );
                std::process::exit(0);
            }
            a if is_discord_url(a) => url = a.to_string(),
            a => eprintln!("[{APP_NAME}] ignoring unknown argument: {a}"),
        }
    }
    url
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    Some(host.rsplit_once('@').map_or(host, |(_, h)| h).split(':').next()?)
}

fn is_discord_url(url: &str) -> bool {
    host_of(url).is_some_and(|h| {
        ["discord.com", "discordapp.com", "discord.gg"]
            .iter()
            .any(|d| h == *d || h.ends_with(&format!(".{d}")))
    })
}

fn download_name(url: &str, suggested: &Path) -> String {
    suggested
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            url.split(['?', '#'])
                .next()?
                .rsplit('/')
                .next()
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "download".into())
        .replace(['/', '\\'], "_")
}

/// `file.png`, then `file (1).png`, `file (2).png`, … so nothing is overwritten.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let _ = fs::create_dir_all(dir);
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

fn load_window_size(data_dir: &Path) -> Option<(f64, f64)> {
    let s = fs::read_to_string(data_dir.join("window")).ok()?;
    let mut it = s.split_whitespace().map(|v| v.parse::<f64>().ok());
    let (w, h) = (it.next()??, it.next()??);
    (w >= 480.0 && h >= 360.0).then_some((w, h))
}

fn save_window_size(data_dir: &Path, w: f64, h: f64) {
    let _ = fs::write(data_dir.join("window"), format!("{w:.0} {h:.0}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_urls() {
        assert!(is_discord_url("https://discord.com/channels/@me"));
        assert!(is_discord_url("https://canary.discord.com/app"));
        assert!(is_discord_url("https://discord.gg/rust"));
        assert!(!is_discord_url("https://notdiscord.com/"));
        assert!(!is_discord_url("https://discord.com.evil.io/"));
        assert!(!is_discord_url("https://evil.io/?https://discord.com"));
        assert!(!is_discord_url("https://discord.com@evil.io/"));
        assert!(!is_discord_url("http://discord.com/"));
    }

    #[test]
    fn download_names() {
        let empty = Path::new("");
        assert_eq!(download_name("https://cdn.discordapp.com/a/b/cat.png?ex=1", empty), "cat.png");
        assert_eq!(download_name("https://x.y/", empty), "download");
        assert_eq!(download_name("https://x.y/a.txt", Path::new("/tmp/real.txt")), "real.txt");
    }
}
