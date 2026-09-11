/// 智能自适应蓝牙轮询器
/// 根据信号强度动态调整扫描频率，确保断连时准确触发锁屏

/// 连续低于阈值的次数阈值，防止信号毛刺误判
const LOW_RSSI_CONFIRM_COUNT: i32 = 3;

pub struct SmartPoller {
    threshold: i32,
    scan_interval: i32,
    active: bool,
    low_count: i32,
}

impl SmartPoller {
    pub fn new(threshold: i32, scan_interval: i32) -> Self {
        Self {
            threshold,
            scan_interval,
            active: false,
            low_count: 0,
        }
    }

    pub fn set_threshold(&mut self, threshold: i32) {
        self.threshold = threshold;
    }

    pub fn set_scan_interval(&mut self, interval: i32) {
        self.scan_interval = interval;
    }

    pub fn start(&mut self) {
        self.active = true;
        self.low_count = 0;
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.low_count = 0;
    }

    /// 根据当前连接状态和 RSSI 返回下次轮询间隔（毫秒）
    /// 返回 (poll_ms, should_trigger_disconnect)
    pub fn tick(&mut self, connected: bool, rssi: i32, rssi_supported: bool) -> (u32, bool) {
        if !self.active {
            return (self.scan_interval as u32 * 1000, false);
        }

        if !connected {
            self.low_count = 0;
            return (self.scan_interval as u32 * 1000, true);
        }

        // RSSI 不支持时跳过阈值判断
        if rssi == 0 || !rssi_supported {
            return (self.scan_interval as u32 * 1000, false);
        }

        // 低信号检测
        if rssi < self.threshold {
            self.low_count += 1;
            log::debug!("低信号计数: {}/{}", self.low_count, LOW_RSSI_CONFIRM_COUNT);
            if self.low_count >= LOW_RSSI_CONFIRM_COUNT {
                self.low_count = 0;
                return (self.scan_interval as u32 * 1000, true);
            }
        } else {
            self.low_count = 0;
        }

        // 正常监控：信号越弱轮询越快
        if rssi >= -70 {
            (self.scan_interval as u32 * 1000, false)
        } else {
            (1000, false)
        }
    }
}
