/// 系统托盘模块
/// 抽象接口 + 多桌面环境适配
///
/// 已适配桌面环境：
/// - KDE Plasma    → StatusNotifierItem D-Bus
/// - Deepin/DDE    → StatusNotifierItem D-Bus
/// - XFCE          → StatusNotifierItem D-Bus
/// - Cinnamon      → StatusNotifierItem D-Bus
/// - MATE          → StatusNotifierItem D-Bus
/// - Unity         → StatusNotifierItem D-Bus
/// - GNOME         → 窗口隐藏回退（GNOME 不支持传统托盘）
/// - 其他          → 自动检测 StatusNotifierWatcher → 有则用 SNI，无则窗口隐藏
mod sni;
mod fallback;

/// 托盘状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Connected,
    Fading,
    Disconnected,
    Paused,
    BleOff,
    Reconnecting,
}

/// 托盘后端 trait — 所有桌面环境实现此接口
pub trait TrayBackend: Send {
    /// 启动托盘服务
    fn start(&mut self) -> bool;
    /// 更新状态显示
    fn update_status(&mut self, state: TrayState, rssi: i32);
    /// 暂停/恢复
    fn set_paused(&mut self, paused: bool);
    /// 停止并清理
    fn stop(&mut self);
}

/// 托盘控制器 — 自动检测桌面环境并选择后端
pub struct TrayIcon {
    backend: Box<dyn TrayBackend>,
}

impl TrayIcon {
    pub fn new() -> Self {
        let backend = detect_backend();
        Self {
            backend,
        }
    }

    pub fn start(&mut self) {
        self.backend.start();
    }

    pub fn update_status(&mut self, state: TrayState, rssi: i32) {
        self.backend.update_status(state, rssi);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.backend.set_paused(paused);
    }

    pub fn stop(&mut self) {
        self.backend.stop();
    }
}

/// 检测桌面环境，选择最佳托盘后端
fn detect_backend() -> Box<dyn TrayBackend> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    let session = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_lowercase();

    log::info!("桌面环境: XDG_CURRENT_DESKTOP={}, XDG_SESSION_TYPE={}", desktop, session);

    // StatusNotifierItem 支持的桌面环境
    // KDE Plasma 原生支持 SNI
    if desktop.contains("kde") || desktop.contains("plasma") {
        log::info!("检测到 KDE/Plasma，使用 StatusNotifierItem 托盘");
        return Box::new(sni::SnITray::new());
    }

    // Deepin/DDE 支持 StatusNotifierItem
    if desktop.contains("deepin") || desktop.contains("dde") {
        log::info!("检测到 Deepin/DDE，使用 StatusNotifierItem 托盘");
        return Box::new(sni::SnITray::new());
    }

    // XFCE 支持 StatusNotifierItem（通过 pluma 或 libappindicator）
    if desktop.contains("xfce") {
        log::info!("检测到 XFCE，使用 StatusNotifierItem 托盘");
        return Box::new(sni::SnITray::new());
    }

    // Cinnamon 支持 StatusNotifierItem
    if desktop.contains("cinnamon") {
        log::info!("检测到 Cinnamon，使用 StatusNotifierItem 托盘");
        return Box::new(sni::SnITray::new());
    }

    // MATE 支持 StatusNotifierItem（通过 mate-panel applet）
    if desktop.contains("mate") {
        log::info!("检测到 MATE，使用 StatusNotifierItem 托盘");
        return Box::new(sni::SnITray::new());
    }

    // Unity 支持 AppIndicator
    if desktop.contains("unity") {
        log::info!("检测到 Unity，使用 StatusNotifierItem 托盘");
        return Box::new(sni::SnITray::new());
    }

    // 尝试检测系统是否支持 StatusNotifierItem
    // 如果 StatusNotifierWatcher 存在，说明系统支持 SNI
    if sni::watcher_exists() {
        log::info!("系统支持 StatusNotifierItem，使用 SNI 托盘");
        return Box::new(sni::SnITray::new());
    }

    // GNOME / Deepin / 其他不支持传统托盘的环境
    // 使用窗口隐藏回退方案
    log::info!("使用窗口隐藏回退方案（GNOME/Deepin/其他）");
    Box::new(fallback::FallbackTray::new())
}
