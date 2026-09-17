/// 回退托盘实现 — 适用于 GNOME / Deepin 等不支持传统托盘的环境
/// 通过窗口隐藏到任务栏实现，用户可从任务栏恢复窗口
use super::{TrayBackend, TrayState};

pub struct FallbackTray {
    paused: bool,
}

impl FallbackTray {
    pub fn new() -> Self {
        Self { paused: false }
    }
}

impl TrayBackend for FallbackTray {
    fn start(&mut self) -> bool {
        log::info!("回退托盘已启动（窗口隐藏模式）");
        true
    }

    fn update_status(&mut self, state: TrayState, rssi: i32) {
        // 回退模式下通过日志通知状态变化
        match state {
            TrayState::Connected => {
                log::debug!("[托盘] 已连接, 信号: {rssi}dBm");
            }
            TrayState::Disconnected => {
                log::debug!("[托盘] 已断开");
            }
            TrayState::BleOff => {
                log::debug!("[托盘] 蓝牙已关闭");
            }
            _ => {}
        }
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn stop(&mut self) {
        log::info!("回退托盘已停止");
    }
}
