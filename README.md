# discord-rs

A lightweight Discord desktop app for Windows, written in Rust. It runs the
official Discord web app inside Windows' built-in WebView2 (the Edge engine)
instead of shipping its own copy of Chromium like the official Electron app,
so the .exe is a couple of MB and uses noticeably less RAM.

## Build on Windows

1. Install Rust: https://rustup.rs (accept the defaults; it will offer to
   install the Visual Studio C++ Build Tools — say yes).
2. Open a terminal in this folder and run:

   ```
   cargo build --release
   ```

3. Your app is at `target\release\discord-rs.exe`. Copy it anywhere and make a
   shortcut to it.

WebView2 already ships with Windows 10 and 11. If it's somehow missing, get the
"Evergreen Bootstrapper" from Microsoft's WebView2 page.

## Features

- Stays logged in between launches (data kept in `%APPDATA%\discord-rs`)
- Voice, video, screen share and desktop notifications allowed
- Unread count shows in the window title
- Discord invite/message links open in the app; all other links open in your
  normal browser
- Downloads go to your Downloads folder (never overwrites existing files)
- Remembers window size

## Shortcuts

| Keys | Action |
|---|---|
| Ctrl+R / F5 | Reload |
| Ctrl+= / Ctrl+- / Ctrl+0 | Zoom in / out / reset |
| F11 | Fullscreen |
| F12 / Ctrl+Shift+I | Developer tools |

## Options

```
discord-rs.exe --ptb       Public Test Build
discord-rs.exe --canary    Canary build
discord-rs.exe https://discord.com/channels/@me   open a specific page
```

To log out completely, delete `%APPDATA%\discord-rs`.

## Note

Discord's terms don't officially cover third-party clients. This one only
displays the official web app unmodified, which is about as low-risk as it
gets, but it isn't endorsed by Discord.
