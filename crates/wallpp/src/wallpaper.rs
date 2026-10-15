use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Checks if a command binary is present in PATH.
pub fn is_command_available(cmd: &str) -> bool {
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let full = dir.join(cmd);
            if full.is_file() {
                return true;
            }
        }
    }
    false
}

/// Checks if a process with the given name is currently running.
fn is_process_running(name: &str) -> bool {
    let output = Command::new("pgrep").arg("-x").arg(name).output();
    matches!(output, Ok(out) if out.status.success())
}

/// Runs a command with the given arguments, returning true if successful.
fn run_cmd(cmd: &str, args: &[&str]) -> bool {
    if !is_command_available(cmd) {
        return false;
    }
    match Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

/// Runs a command and feeds input to its stdin.
fn run_cmd_with_stdin(cmd: &str, args: &[&str], input: &str) -> bool {
    if !is_command_available(cmd) {
        return false;
    }
    let mut child = match Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }

    match child.wait() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

/// Desktop environments and window managers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desktop {
    Gnome,
    Kde,
    Cinnamon,
    Mate,
    Xfce,
    Lxqt,
    Lxde,
    Deepin,
    Sway,
    Hyprland,
    Cosmic,
    Awesome,
    Fluxbox,
    Unknown,
}

/// Detects the desktop environment or window manager.
pub fn detect_desktop() -> Desktop {
    if let Ok(desktop) = env::var("XDG_CURRENT_DESKTOP") {
        let lower = desktop.to_lowercase();
        if lower.contains("gnome") || lower.contains("unity") || lower.contains("budgie") {
            return Desktop::Gnome;
        } else if lower.contains("kde") {
            return Desktop::Kde;
        } else if lower.contains("cinnamon") {
            return Desktop::Cinnamon;
        } else if lower.contains("mate") {
            return Desktop::Mate;
        } else if lower.contains("xfce") {
            return Desktop::Xfce;
        } else if lower.contains("lxqt") {
            return Desktop::Lxqt;
        } else if lower.contains("lxde") {
            return Desktop::Lxde;
        } else if lower.contains("deepin") {
            return Desktop::Deepin;
        } else if lower.contains("sway") {
            return Desktop::Sway;
        } else if lower.contains("hyprland") {
            return Desktop::Hyprland;
        } else if lower.contains("cosmic") {
            return Desktop::Cosmic;
        } else if lower.contains("fluxbox") {
            return Desktop::Fluxbox;
        }
    }

    if let Ok(session) = env::var("XDG_SESSION_DESKTOP") {
        let lower = session.to_lowercase();
        if lower.contains("awesome") {
            return Desktop::Awesome;
        }
    }

    Desktop::Unknown
}

/// Attempts to set wallpaper on Wayland/wlroots compositors.
fn set_wlroots(file_path: &Path) -> Option<&'static str> {
    let path_str = file_path.to_string_lossy();

    if is_process_running("wpaperd") && run_cmd("wpaperctl", &["set", &path_str]) {
        return Some("wpaperctl");
    }
    if is_process_running("awww-daemon") && run_cmd("awww", &["img", &path_str]) {
        return Some("awww");
    }
    if is_command_available("swaybg") {
        // Spawn new swaybg and kill existing ones to avoid flicker.
        let old_pids = Command::new("pidof")
            .arg("swaybg")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        if let Ok(mut child) = Command::new("swaybg")
            .args(["-i", &path_str, "-m", "fill"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            // Detach swaybg child process so it keeps running.
            std::thread::spawn(move || {
                let _ = child.wait();
            });

            if let Some(pids) = old_pids {
                std::thread::sleep(std::time::Duration::from_millis(500));
                for pid in pids.split_whitespace() {
                    let _ = Command::new("kill").arg(pid).status();
                }
            }
            return Some("swaybg");
        }
    }

    None
}

/// Sets the desktop wallpaper for the detected desktop environment or window manager.
/// Returns the name of the method/tool that was successfully used.
pub fn set_wallpaper(image_path: &Path) -> Result<String> {
    let abs_path = fs::canonicalize(image_path)
        .with_context(|| format!("Invalid wallpaper image path: {:?}", image_path))?;
    let path_str = abs_path.to_string_lossy();
    let file_uri = format!("file://{}", path_str);

    let de = detect_desktop();

    match de {
        Desktop::Gnome => {
            if is_command_available("gsettings") {
                let s1 = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "org.gnome.desktop.background",
                        "picture-uri",
                        &file_uri,
                    ],
                );
                let s2 = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "org.gnome.desktop.background",
                        "picture-uri-dark",
                        &file_uri,
                    ],
                );
                let _ = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "org.gnome.desktop.background",
                        "picture-options",
                        "zoom",
                    ],
                );
                if s1 || s2 {
                    return Ok("gsettings (gnome)".to_string());
                }
            }
        }
        Desktop::Cinnamon => {
            if is_command_available("gsettings") {
                let s = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "org.cinnamon.desktop.background",
                        "picture-uri",
                        &file_uri,
                    ],
                );
                let _ = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "org.cinnamon.desktop.background",
                        "picture-options",
                        "zoom",
                    ],
                );
                if s {
                    return Ok("gsettings (cinnamon)".to_string());
                }
            }
        }
        Desktop::Mate => {
            if is_command_available("gsettings") {
                let s = run_cmd(
                    "gsettings",
                    &["set", "org.mate.background", "picture-filename", &path_str],
                );
                let _ = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "org.mate.desktop.background",
                        "picture-options",
                        "zoom",
                    ],
                );
                if s {
                    return Ok("gsettings (mate)".to_string());
                }
            }
        }
        Desktop::Deepin => {
            if is_command_available("gsettings") {
                let s = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "com.deepin.wrap.gnome.desktop.background",
                        "picture-uri",
                        &file_uri,
                    ],
                );
                let _ = run_cmd(
                    "gsettings",
                    &[
                        "set",
                        "com.deepin.wrap.gnome.desktop.background",
                        "picture-options",
                        "zoom",
                    ],
                );
                if s {
                    return Ok("gsettings (deepin)".to_string());
                }
            }
        }
        Desktop::Kde => {
            if is_command_available("dbus-send") {
                let script = format!(
                    r#"
                    let supportedPlugins = Array('org.kde.image', 'a2n.blur');
                    let allDesktops = desktops();
                    for (let d of allDesktops) {{
                        if (supportedPlugins.includes(d.wallpaperPlugin)) {{
                            d.currentConfigGroup = Array('Wallpaper', d.wallpaperPlugin, 'General');
                            d.writeConfig('Image', '{}');
                        }}
                    }}
                    "#,
                    file_uri
                );
                let ok = run_cmd(
                    "dbus-send",
                    &[
                        "--session",
                        "--type=method_call",
                        "--dest=org.kde.plasmashell",
                        "/PlasmaShell",
                        "org.kde.PlasmaShell.evaluateScript",
                        &format!("string:{}", script),
                    ],
                );
                if ok {
                    return Ok("plasma-dbus".to_string());
                }
            }
        }
        Desktop::Xfce => {
            if is_command_available("xfconf-query") {
                let output = Command::new("xfconf-query")
                    .args(["-c", "xfce4-desktop", "-p", "/backdrop", "-l"])
                    .output();
                if let Ok(out) = output {
                    let out_str = String::from_utf8_lossy(&out.stdout);
                    let mut found = false;
                    for line in out_str.lines() {
                        if line.ends_with("last-image") || line.ends_with("image-path") {
                            run_cmd(
                                "xfconf-query",
                                &["-c", "xfce4-desktop", "-p", line, "-s", &path_str],
                            );
                            found = true;
                        }
                    }
                    if found {
                        return Ok("xfconf-query".to_string());
                    }
                }
            }
        }
        Desktop::Sway => {
            if let Some(m) = set_wlroots(&abs_path) {
                return Ok(m.to_string());
            }
            if is_command_available("swaymsg")
                && run_cmd("swaymsg", &["output", "*", "bg", &path_str, "fill"])
            {
                return Ok("swaymsg".to_string());
            }
        }
        Desktop::Hyprland => {
            if let Some(m) = set_wlroots(&abs_path) {
                return Ok(m.to_string());
            }
            if is_process_running("hyprpaper") && is_command_available("hyprctl") {
                let monitors = Command::new("hyprctl").args(["monitors", "all"]).output();
                if let Ok(out) = monitors {
                    let out_str = String::from_utf8_lossy(&out.stdout);
                    let mut set_any = false;
                    for line in out_str.lines() {
                        if line.starts_with("Monitor ") {
                            if let Some(m_name) = line.split_whitespace().nth(1) {
                                let arg = format!("{},{}", m_name, path_str);
                                if run_cmd("hyprctl", &["hyprpaper", "wallpaper", &arg]) {
                                    set_any = true;
                                }
                            }
                        }
                    }
                    if set_any {
                        return Ok("hyprpaper".to_string());
                    }
                }
            }
        }
        Desktop::Lxqt => {
            if is_command_available("pcmanfm-qt")
                && run_cmd("pcmanfm-qt", &["--set-wallpaper", &path_str])
            {
                return Ok("pcmanfm-qt".to_string());
            }
        }
        Desktop::Lxde => {
            if is_command_available("pcmanfm")
                && run_cmd("pcmanfm", &["--set-wallpaper", &path_str])
            {
                return Ok("pcmanfm".to_string());
            }
        }
        Desktop::Cosmic => {
            let cosmic_path = env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".config/cosmic/com.system76.CosmicBackground/v1/all");
            if cosmic_path.exists() {
                if let Ok(content) = fs::read_to_string(&cosmic_path) {
                    let mut new_lines = Vec::new();
                    for line in content.lines() {
                        if line.trim().starts_with("source: Path(")
                            || line.trim().starts_with("source: Color(")
                        {
                            let indent = line
                                .chars()
                                .take_while(|c| c.is_whitespace())
                                .collect::<String>();
                            new_lines.push(format!("{}source: Path(\"{}\"),", indent, path_str));
                        } else {
                            new_lines.push(line.to_string());
                        }
                    }
                    let _ = fs::write(&cosmic_path, new_lines.join("\n"));
                    return Ok("cosmic-config".to_string());
                }
            }
        }
        Desktop::Awesome => {
            let lua_code = format!(
                "for s in screen do require(\"gears\").wallpaper.maximized(\"{}\", s) end",
                path_str
            );
            if is_command_available("awesome-client")
                && run_cmd_with_stdin("awesome-client", &[], &lua_code)
            {
                return Ok("awesome-client".to_string());
            }
        }
        Desktop::Fluxbox => {
            if is_command_available("fbsetbg") && run_cmd("fbsetbg", &[&path_str]) {
                return Ok("fbsetbg".to_string());
            }
        }
        Desktop::Unknown => {}
    }

    // Wayland compositor fallback.
    if let Some(m) = set_wlroots(&abs_path) {
        return Ok(m.to_string());
    }

    // Generic X11 fallbacks (feh, nitrogen).
    if is_command_available("feh") && run_cmd("feh", &["--bg-fill", &path_str]) {
        return Ok("feh".to_string());
    }
    if is_command_available("nitrogen")
        && run_cmd("nitrogen", &["--set-zoom-fill", "--save", &path_str])
    {
        return Ok("nitrogen".to_string());
    }

    bail!(
        "Failed to set wallpaper: no supported desktop environment tool found in PATH for desktop {:?}",
        de
    )
}
