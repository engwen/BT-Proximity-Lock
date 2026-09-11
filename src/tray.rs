/// 系统托盘模块
/// 当前为 stub 实现（需要 libdbus-1-dev 才能使用 ksni crate）
/// TODO: 安装 libdbus-1-dev 后可启用 ksni 实现

/// 托盘状态
#[derive(Debug, Clone, PartialEq)]
pub enum TrayState {
    Connected,
    Fading,
    Disconnected,
    Paused,
    BleOff,
    Reconnecting,
}

pub struct TrayIcon;

impl TrayIcon {
    pub fn new() -> Self {
        Self
    }

    pub fn start(&mut self) {
        log::info!("系统托盘（stub 模式，需安装 libdbus-1-dev 启用完整功能）");
    }

    pub fn update_status(&self, _state: TrayState, _rssi: i32) {}

    pub fn set_paused(&self, _paused: bool) {}

    pub fn set_callbacks(
        &mut self,
        _on_pause_resume: Box<dyn Fn() + Send>,
        _on_quit: Box<dyn Fn() + Send>,
    ) {
    }

    pub fn stop(&mut self) {
        log::info!("系统托盘已停止");
    }
}
