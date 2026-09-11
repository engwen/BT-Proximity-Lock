/// 屏幕锁定模块
/// 支持多种桌面环境（GNOME/KDE/XFCE/Cinnamon/MATE）的锁屏
use std::process::Command;

/// 获取系统空闲时间（毫秒），通过 GNOME Mutter IdleMonitor D-Bus
pub fn get_idle_ms() -> u64 {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.gnome.Mutter.IdleMonitor",
            "--object-path",
            "/org/gnome/Mutter/IdleMonitor/Core",
            "--method",
            "org.gnome.Mutter.IdleMonitor.GetIdletime",
        ])
        .output();
    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        // 解析 "uint64 12345" 格式
        for part in stdout.split_whitespace() {
            if let Ok(ms) = part.parse::<u64>() {
                return ms;
            }
        }
    }
    0
}

/// 锁定屏幕，自动探测桌面环境并选择最佳方式
pub fn lock_screen() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();

    let methods: Vec<fn() -> bool> = if desktop.contains("kde") || desktop.contains("plasma") {
        vec![try_kde_lock, try_loginctl, try_xdg_screensaver]
    } else if desktop.contains("xfce") {
        vec![try_xfce_lock, try_loginctl, try_xdg_screensaver]
    } else if desktop.contains("cinnamon") {
        vec![try_cinnamon_lock, try_loginctl, try_gnome_screensaver]
    } else if desktop.contains("mate") {
        vec![try_mate_lock, try_loginctl, try_gnome_screensaver]
    } else {
        vec![try_loginctl, try_gnome_screensaver, try_xdg_screensaver]
    };

    for method in methods {
        if method() {
            return true;
        }
    }

    log::error!("所有锁屏方式均失败，请检查系统环境");
    false
}

/// 解锁当前登录会话
pub fn unlock_session() -> bool {
    // 方式1: loginctl unlock-session
    if run_cmd(&["loginctl", "unlock-session"]) {
        log::info!("会话已解锁 (loginctl)");
        return true;
    }
    // 方式2: GNOME Screensaver D-Bus Unlock
    if run_cmd(&[
        "dbus-send",
        "--session",
        "--type=method_call",
        "--dest=org.gnome.ScreenSaver",
        "/org/gnome/ScreenSaver",
        "org.gnome.ScreenSaver.Unlock",
    ]) {
        log::info!("会话已解锁 (gnome-screensaver)");
        return true;
    }
    log::debug!("解锁失败：loginctl 和 D-Bus 均不可用");
    false
}

fn run_cmd(cmd: &[&str]) -> bool {
    match Command::new(cmd[0]).args(&cmd[1..]).output() {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

fn try_loginctl() -> bool {
    if run_cmd(&["loginctl", "lock-session"]) {
        log::info!("屏幕已锁定 (loginctl)");
        return true;
    }
    false
}

fn try_gnome_screensaver() -> bool {
    if run_cmd(&[
        "dbus-send",
        "--session",
        "--type=method_call",
        "--dest=org.gnome.ScreenSaver",
        "/org/gnome/ScreenSaver",
        "org.gnome.ScreenSaver.Lock",
    ]) {
        log::info!("屏幕已锁定 (gnome-screensaver)");
        return true;
    }
    false
}

fn try_xdg_screensaver() -> bool {
    if run_cmd(&["xdg-screensaver", "lock"]) {
        log::info!("屏幕已锁定 (xdg-screensaver)");
        return true;
    }
    false
}

fn try_kde_lock() -> bool {
    if run_cmd(&["qdbus", "org.kde.screensaver", "/ScreenSaver", "Lock"]) {
        log::info!("屏幕已锁定 (KDE)");
        return true;
    }
    if run_cmd(&[
        "dbus-send",
        "--session",
        "--type=method_call",
        "--dest=org.kde.screensaver",
        "/ScreenSaver",
        "org.kde.screensaver.Lock",
    ]) {
        log::info!("屏幕已锁定 (KDE D-Bus)");
        return true;
    }
    false
}

fn try_xfce_lock() -> bool {
    if run_cmd(&["xfce4-screensaver-command", "--lock"]) {
        log::info!("屏幕已锁定 (XFCE)");
        return true;
    }
    false
}

fn try_cinnamon_lock() -> bool {
    if run_cmd(&[
        "dbus-send",
        "--session",
        "--type=method_call",
        "--dest=org.cinnamon.ScreenSaver",
        "/org/cinnamon/ScreenSaver",
        "org.cinnamon.ScreenSaver.Lock",
    ]) {
        log::info!("屏幕已锁定 (Cinnamon)");
        return true;
    }
    false
}

fn try_mate_lock() -> bool {
    if run_cmd(&["mate-screensaver-command", "--lock"]) {
        log::info!("屏幕已锁定 (MATE)");
        return true;
    }
    false
}
