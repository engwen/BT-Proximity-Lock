/// 蓝牙监控模块
/// 通过 BlueZ D-Bus API 监控手机蓝牙连接状态和信号强度
use std::collections::VecDeque;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

const BLUEZ_BUS: &str = "org.bluez";
const DEVICE_IF: &str = "org.bluez.Device1";
const ADAPTER_IF: &str = "org.bluez.Adapter1";
const PROPS_IF: &str = "org.freedesktop.DBus.Properties";

/// 蓝牙监控事件
#[derive(Debug, Clone)]
pub enum BtEvent {
    Connected,
    Disconnected,
    RssiChanged(i32),
    AdapterPoweredOff,
    AdapterPoweredOn,
}

/// 蓝牙监控器
pub struct BluetoothMonitor {
    pub mac: String,
    pub rssi_threshold: i32,
    pub connected: bool,
    pub current_rssi: i32,
    pub device_path: String,
    pub adapter_powered: bool,
    pub filtered_rssi: i32,
    pub rssi_supported: bool,
    rssi_samples: VecDeque<i32>,
    // 连接状态去抖动
    conn_confirm_count: i32,
    pending_conn: Option<bool>,
    // 事件通道
    event_tx: Option<mpsc::Sender<BtEvent>>,
}

struct DeviceProps {
    connected: Option<bool>,
    rssi: Option<i32>,
}

impl BluetoothMonitor {
    pub fn new(mac: &str, rssi_threshold: i32) -> Self {
        Self {
            mac: mac.to_uppercase().replace('-', ":"),
            rssi_threshold,
            connected: false,
            current_rssi: -100,
            device_path: String::new(),
            adapter_powered: true,
            filtered_rssi: -100,
            rssi_supported: true,
            rssi_samples: VecDeque::with_capacity(8),
            conn_confirm_count: 0,
            pending_conn: None,
            event_tx: None,
        }
    }

    pub fn set_event_sender(&mut self, tx: mpsc::Sender<BtEvent>) {
        self.event_tx = Some(tx);
    }

    fn send_event(&self, event: BtEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// 更新 RSSI，应用滑动中位数滤波
    fn update_rssi(&mut self, raw: i32) {
        if raw == 0 || raw == -100 {
            if self.current_rssi == -100 {
                self.current_rssi = 0;
                self.filtered_rssi = 0;
                self.rssi_supported = false;
                log::warn!("适配器不支持经典蓝牙 RSSI");
            }
            return;
        }
        self.rssi_supported = true;
        self.current_rssi = raw;
        if raw > -100 {
            self.rssi_samples.push_back(raw);
            if self.rssi_samples.len() > 8 {
                self.rssi_samples.pop_front();
            }
        }
        if !self.rssi_samples.is_empty() {
            let mut sorted: Vec<i32> = self.rssi_samples.iter().copied().collect();
            sorted.sort();
            self.filtered_rssi = sorted[sorted.len() / 2];
        } else {
            self.filtered_rssi = raw;
        }
    }

    /// 初始化：查找设备路径并获取初始状态
    pub fn setup(&mut self) -> bool {
        self.device_path = self.find_device();
        if self.device_path.is_empty() {
            log::warn!("未找到设备 {}，等待适配器启动后重试", self.mac);
            return false;
        }
        let props = self.get_device_props();
        self.connected = props.connected.unwrap_or(false);
        self.update_rssi(props.rssi.unwrap_or(-100));
        log::info!(
            "蓝牙监控就绪 - 设备: {} 连接: {} RSSI: {}",
            self.device_path,
            self.connected,
            self.current_rssi
        );
        true
    }

    /// 手动轮询设备状态（作为 D-Bus 信号的补充）
    pub fn poll_status(&mut self) {
        // 检测适配器状态
        let new_powered = self.get_adapter_powered();
        if let Some(powered) = new_powered {
            if powered != self.adapter_powered {
                self.adapter_powered = powered;
                if !powered {
                    log::info!("蓝牙适配器已关闭");
                    self.send_event(BtEvent::AdapterPoweredOff);
                    return;
                } else {
                    log::info!("蓝牙适配器已开启");
                    self.send_event(BtEvent::AdapterPoweredOn);
                }
            }
        }
        if !self.adapter_powered {
            return;
        }
        if self.device_path.is_empty() {
            self.device_path = self.find_device();
            if self.device_path.is_empty() {
                // 使用 bluetoothctl 兜底
                let connected = self.get_connected_via_bluetoothctl();
                if let Some(c) = connected {
                    if c != self.connected {
                        self.connected = c;
                        if c {
                            self.send_event(BtEvent::Connected);
                        } else {
                            self.send_event(BtEvent::Disconnected);
                        }
                    }
                }
                return;
            }
        }
        let props = self.get_device_props();
        let new_connected = props.connected.unwrap_or(false);
        let new_rssi = props.rssi.unwrap_or(-100);
        // 去抖动处理
        if new_connected == self.connected {
            self.pending_conn = None;
            self.conn_confirm_count = 0;
        } else {
            if Some(new_connected) == self.pending_conn {
                self.conn_confirm_count += 1;
            } else {
                self.pending_conn = Some(new_connected);
                self.conn_confirm_count = 1;
            }
            let threshold = if new_connected { 3 } else { 2 };
            if self.conn_confirm_count >= threshold {
                let old = self.connected;
                self.connected = new_connected;
                self.pending_conn = None;
                self.conn_confirm_count = 0;
                log::info!("[轮询] 连接状态确认变化: {old} -> {new_connected}");
                if new_connected {
                    self.send_event(BtEvent::Connected);
                } else {
                    self.send_event(BtEvent::Disconnected);
                }
            }
        }
        self.update_rssi(new_rssi);
        // 只在RSSI值实际变化时发送事件，避免过于频繁的事件
        if new_rssi != self.current_rssi {
            self.send_event(BtEvent::RssiChanged(new_rssi));
        }
    }

    /// 启动连接子进程（LE + 经典蓝牙双路并行），返回 (le_proc, br_proc)
    pub fn start_connect_process(&self, timeout_sec: u64) -> (Option<Child>, Option<Child>) {
        let mac = self.mac.clone();
        log::info!("启动连接子进程: {mac}（超时{timeout_sec}秒）");
        // 方式1: bluetoothctl connect（LE）
        let le_proc = Command::new("timeout")
            .arg(format!("{timeout_sec}"))
            .args(["bluetoothctl", "connect", &mac])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok();
        // 方式2: gdbus Device1.Connect（经典蓝牙）
        let dbus_path = format!("/org/bluez/hci0/dev_{}", mac.replace(':', "_"));
        let br_proc = Command::new("timeout")
            .arg(format!("{}", timeout_sec + 5))
            .args([
                "gdbus",
                "call",
                "--system",
                "--dest",
                BLUEZ_BUS,
                "--object-path",
                &dbus_path,
                "--method",
                &format!("{DEVICE_IF}.Connect"),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok();
        (le_proc, br_proc)
    }

    /// 实时查询设备连接状态
    pub fn check_connected_now(&mut self) -> bool {
        let props = self.get_device_props();
        props.connected.unwrap_or(false)
    }

    /// 断开指定蓝牙外设
    pub fn disconnect_peripheral(&self, mac: &str) -> bool {
        let path_suffix = mac.replace(':', "_");
        let device_path = format!("/org/bluez/hci0/dev_{path_suffix}");
        let result = Command::new("gdbus")
            .args([
                "call",
                "--system",
                "--dest",
                BLUEZ_BUS,
                "--object-path",
                &device_path,
                "--method",
                &format!("{DEVICE_IF}.Disconnect"),
            ])
            .output();
        match result {
            Ok(out) => {
                if out.status.success() {
                    log::info!("已断开外设: {mac}");
                    true
                } else {
                    false
                }
            }
            Err(e) => {
                log::error!("断开外设异常: {e}");
                false
            }
        }
    }

    // ── 内部方法 ──────────────────────────────────────────────────────────────

    fn find_device(&self) -> String {
        let path_suffix = self.mac.replace(':', "_");
        let candidate = format!("/org/bluez/hci0/dev_{path_suffix}");
        let output = Command::new("gdbus")
            .args([
                "call",
                "--system",
                "--dest",
                BLUEZ_BUS,
                "--object-path",
                &candidate,
                "--method",
                &format!("{PROPS_IF}.GetAll"),
                DEVICE_IF,
            ])
            .output();
        if let Ok(output) = output {
            if output.status.success() {
                return candidate;
            }
        }
        self.find_device_om()
    }

    fn find_device_om(&self) -> String {
        let output = Command::new("gdbus")
            .args([
                "call",
                "--system",
                "--dest",
                BLUEZ_BUS,
                "--object-path",
                "/",
                "--method",
                "org.freedesktop.DBus.ObjectManager.GetManagedObjects",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let path_suffix = self.mac.replace(':', "_");
                let target = format!("dev_{path_suffix}");
                for line in stdout.lines() {
                    if line.contains(&target) {
                        // 提取路径
                        if let Some(start) = line.find("'/org/bluez/") {
                            let rest = &line[start + 1..];
                            if let Some(end) = rest.find('\'') {
                                return rest[..end].to_string();
                            }
                        }
                    }
                }
            }
        }
        String::new()
    }

    fn get_device_props(&self) -> DeviceProps {
        let mut props = DeviceProps {
            connected: None,
            rssi: None,
        };
        // Connected: D-Bus + bluetoothctl 双重检查
        let connected_dbus = self.get_connected_via_dbus();
        let connected_ctl = self.get_connected_via_bluetoothctl();
        if let (Some(dbus), Some(ctl)) = (connected_dbus, connected_ctl) {
            if dbus != ctl {
                log::warn!(
                    "连接状态检测不一致: D-Bus={dbus}, bluetoothctl={ctl}，采用bluetoothctl"
                );
                props.connected = Some(ctl);
            } else {
                props.connected = Some(dbus);
            }
        } else if let Some(c) = connected_ctl {
            props.connected = Some(c);
        } else {
            props.connected = connected_dbus;
        }
        // RSSI via HCI
        if let Some(handle) = self.get_conn_handle() {
            if let Some(rssi) = self.read_rssi_hci(handle) {
                props.rssi = Some(rssi);
            }
        }
        props
    }

    fn get_connected_via_dbus(&self) -> Option<bool> {
        if self.device_path.is_empty() {
            return None;
        }
        let output = Command::new("gdbus")
            .args([
                "call",
                "--system",
                "--dest",
                BLUEZ_BUS,
                "--object-path",
                &self.device_path,
                "--method",
                &format!("{PROPS_IF}.GetAll"),
                DEVICE_IF,
            ])
            .output();
        match output {
            Ok(out) => {
                if !out.status.success() {
                    return None;
                }
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.contains("'Connected': <true>")
                    || stdout.contains("\"Connected\": <true>")
                {
                    Some(true)
                } else if stdout.contains("'Connected': <false>")
                    || stdout.contains("\"Connected\": <false>")
                {
                    Some(false)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    fn get_connected_via_bluetoothctl(&self) -> Option<bool> {
        let mac = self.mac.clone();
        let output = Command::new("bluetoothctl").args(["info", &mac]).output();
        match output {
            Ok(out) => {
                if !out.status.success() {
                    return None;
                }
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.contains("Connected: yes") {
                    Some(true)
                } else if stdout.contains("Connected: no") {
                    Some(false)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    fn get_conn_handle(&self) -> Option<i32> {
        let output = Command::new("hcitool").arg("con").output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut le_handle: Option<i32> = None;
        let mut acl_handle: Option<i32> = None;
        for line in stdout.lines() {
            if line.to_lowercase().contains(&self.mac.to_lowercase()) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(idx) = parts.iter().position(|p| *p == "handle") {
                    if idx + 1 < parts.len() {
                        if let Ok(h) = parts[idx + 1].parse::<i32>() {
                            if line.contains("< LE") {
                                le_handle = Some(h);
                            } else {
                                acl_handle = Some(h);
                            }
                        }
                    }
                }
            }
        }
        le_handle.or(acl_handle)
    }

    fn read_rssi_hci(&self, handle: i32) -> Option<i32> {
        let h = handle & 0xFFF;
        let cmd = format!("0x{:02x}", h & 0xFF);
        let cmd2 = format!("0x{:02x}", (h >> 8) & 0xFF);
        let output = Command::new("hcitool")
            .args(["cmd", "0x05", "0x0005", &cmd, &cmd2])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if !line.starts_with("> HCI Event:") && !line.starts_with('>') {
                let bytes: Vec<&str> = line.split_whitespace().collect();
                if bytes.len() >= 7 {
                    if let Ok(rssi_byte) = u8::from_str_radix(bytes.last()?, 16) {
                        let rssi = if rssi_byte > 127 {
                            rssi_byte as i32 - 256
                        } else {
                            rssi_byte as i32
                        };
                        return Some(rssi);
                    }
                }
            }
        }
        None
    }

    fn get_adapter_powered(&self) -> Option<bool> {
        let adapter_path = self.get_adapter_path();
        let output = Command::new("gdbus")
            .args([
                "call",
                "--system",
                "--dest",
                BLUEZ_BUS,
                "--object-path",
                &adapter_path,
                "--method",
                &format!("{PROPS_IF}.GetAll"),
                ADAPTER_IF,
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("'Powered': <true>") || stdout.contains("\"Powered\": <true>") {
            Some(true)
        } else if stdout.contains("'Powered': <false>") || stdout.contains("\"Powered\": <false>") {
            Some(false)
        } else {
            None
        }
    }

    fn get_adapter_path(&self) -> String {
        if !self.device_path.is_empty() {
            if let Some(idx) = self.device_path.find("/dev_") {
                return self.device_path[..idx].to_string();
            }
        }
        "/org/bluez/hci0".into()
    }
}
