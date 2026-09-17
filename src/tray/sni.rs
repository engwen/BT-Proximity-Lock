/// StatusNotifierItem D-Bus 托盘实现
/// 兼容 KDE Plasma / XFCE / Cinnamon / MATE / Unity
use std::sync::{Arc, Mutex};
use std::process::Command;
use zbus::connection::Builder;
use zbus::interface;
use tokio;

use super::{TrayBackend, TrayState};

/// 检查 StatusNotifierWatcher 是否存在
pub fn watcher_exists() -> bool {
    let output = Command::new("gdbus")
        .args([
            "call", "--session",
            "--dest", "org.kde.StatusNotifierWatcher",
            "--object-path", "/StatusNotifierWatcher",
            "--method", "org.freedesktop.DBus.Properties.Get",
            "org.kde.StatusNotifierWatcher", "ProtocolVersion",
        ])
        .output();
    match output {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

/// D-Bus 上共享的托盘状态
struct SnIState {
    status: TrayState,
    rssi: i32,
    paused: bool,
}

/// StatusNotifierItem D-Bus 接口实现
struct SnIItem {
    state: Arc<Mutex<SnIState>>,
    bus_name: String,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl SnIItem {
    /// StatusNotifierItem 协议属性
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "bt-proximity-lock"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        "BT Proximity Lock"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        let state = self.state.lock().unwrap();
        match state.status {
            TrayState::Connected => "Active",
            TrayState::Fading => "NeedsAttention",
            TrayState::Disconnected => "NeedsAttention",
            TrayState::Paused => "Passive",
            TrayState::BleOff => "NeedsAttention",
            TrayState::Reconnecting => "NeedsAttention",
        }
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        let state = self.state.lock().unwrap();
        if state.paused {
            return "bluetooth-disabled";
        }
        match state.status {
            TrayState::Connected => "bluetooth-active",
            TrayState::Fading => "bluetooth-disconnected",
            TrayState::Disconnected => "bluetooth-disconnected",
            TrayState::Paused => "bluetooth-disabled",
            TrayState::BleOff => "bluetooth-disabled",
            TrayState::Reconnecting => "bluetooth-disconnected",
        }
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (i32, String, String, String) {
        let state = self.state.lock().unwrap();
        let (icon, summary, body) = match state.status {
            TrayState::Connected => (
                "bluetooth-active",
                "已连接".to_string(),
                format!("信号: {} dBm", state.rssi),
            ),
            TrayState::Fading => (
                "bluetooth-disconnected",
                "信号减弱".to_string(),
                format!("信号: {} dBm", state.rssi),
            ),
            TrayState::Disconnected => (
                "bluetooth-disconnected",
                "未连接".to_string(),
                "蓝牙设备已断开".to_string(),
            ),
            TrayState::Paused => (
                "bluetooth-disabled",
                "监控已暂停".to_string(),
                String::new(),
            ),
            TrayState::BleOff => (
                "bluetooth-disabled",
                "蓝牙已关闭".to_string(),
                String::new(),
            ),
            TrayState::Reconnecting => (
                "bluetooth-disconnected",
                "正在重连".to_string(),
                String::new(),
            ),
        };
        (0, icon.to_string(), summary, body)
    }

    #[zbus(property)]
    fn menu(&self) -> zbus::zvariant::OwnedObjectPath {
        // 不注册菜单，使用空路径
        zbus::zvariant::ObjectPath::from_str_unchecked("/").into()
    }

    /// Activate 方法 — 点击托盘图标时调用
    async fn activate(&self, _x: i32, _y: i32) {
        log::info!("托盘图标被点击");
        // 通知主窗口显示
    }

    /// SecondaryActivate 方法 — 中键点击
    async fn secondary_activate(&self, _x: i32, _y: i32) {
        log::info!("托盘图标中键点击");
    }

    /// Scroll 方法 — 滚轮
    async fn scroll(&self, _delta: i32, _orientation: &str) {}
}

/// StatusNotifierItem 托盘后端
pub struct SnITray {
    state: Arc<Mutex<SnIState>>,
    rt: Option<tokio::runtime::Runtime>,
    bus_name: String,
}

impl SnITray {
    pub fn new() -> Self {
        let id = std::process::id();
        // D-Bus well-known name 不允许 '-'，用 '_' 替代
        let bus_name = format!("org.kde.StatusNotifierItem_{}_{}", id, id);
        Self {
            state: Arc::new(Mutex::new(SnIState {
                status: TrayState::Disconnected,
                rssi: -100,
                paused: false,
            })),
            rt: None,
            bus_name,
        }
    }
}

impl TrayBackend for SnITray {
    fn start(&mut self) -> bool {
        let state = self.state.clone();
        let bus_name = self.bus_name.clone();
        let bus_name_log = bus_name.clone();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .ok();

        if let Some(rt) = rt {
            rt.spawn(async move {
                if let Err(e) = run_sni_service(state, &bus_name).await {
                    log::error!("StatusNotifierItem D-Bus 服务启动失败: {e}");
                }
            });
            self.rt = Some(rt);
            log::info!("StatusNotifierItem 托盘已启动: {bus_name_log}");
            true
        } else {
            log::error!("无法创建 tokio 运行时");
            false
        }
    }

    fn update_status(&mut self, state: TrayState, rssi: i32) {
        if let Ok(mut s) = self.state.lock() {
            s.status = state;
            s.rssi = rssi;
        }
    }

    fn set_paused(&mut self, paused: bool) {
        if let Ok(mut s) = self.state.lock() {
            s.paused = paused;
        }
    }

    fn stop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
        log::info!("StatusNotifierItem 托盘已停止");
    }
}

/// 运行 SNI D-Bus 服务
async fn run_sni_service(
    state: Arc<Mutex<SnIState>>,
    bus_name: &str,
) -> zbus::Result<()> {
    let item = SnIItem {
        state: state.clone(),
        bus_name: bus_name.to_string(),
    };

    let conn = Builder::session()?
        .serve_at(format!("/{}/StatusNotifierItem", bus_name.replace('.', "/")), item)?
        .build()
        .await?;

    // 请求 Well-Known 名称
    if let Err(e) = conn.request_name(bus_name).await {
        log::warn!("请求 D-Bus 名称 {bus_name} 失败: {e}");
    }

    // 注册到 StatusNotifierWatcher
    register_with_watcher(bus_name, &conn).await;

    // 保持服务运行
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

/// 注册到 StatusNotifierWatcher
async fn register_with_watcher(bus_name: &str, conn: &zbus::Connection) {
    let proxy = zbus::Proxy::new(
        &conn,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )
    .await;

    if let Ok(proxy) = proxy {
        let result: zbus::Result<()> = proxy
            .call_method("RegisterStatusNotifierItem", &(bus_name,))
            .await
            .map(|_| ());

        match result {
            Ok(()) => log::info!("已注册到 StatusNotifierWatcher: {bus_name}"),
            Err(e) => log::warn!("注册 StatusNotifierItem 失败: {e}"),
        }
    } else {
        log::warn!("StatusNotifierWatcher 不可用");
    }
}
