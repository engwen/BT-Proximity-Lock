mod bluetooth_monitor;
/// BT Proximity Lock - 主控制器：状态机 + GTK4 界面 + 蓝牙监控 + 系统托盘
mod autostart;
mod config;
mod i18n;
mod locker;
mod log_capture;
mod poller;
mod scanner;
mod tray;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use gtk::prelude::*;
use gtk::{self, glib, Application, ApplicationWindow, Box as GtkBox};

use bluetooth_monitor::{BluetoothMonitor, BtEvent};
use config::Config;
use i18n::{detect_language, set_lang, tr, tr_fmt};
use poller::SmartPoller;
use tray::TrayIcon;

// ── 常量 ──────────────────────────────────────────────────────────────────
const MAX_CONNECT_ATTEMPTS: i32 = 3;
const CONNECT_TIMEOUT_SEC: u64 = 5;
const CONNECT_ATTEMPT_INTERVAL_SEC: u32 = 5;
const RETRY_BASE_INTERVAL_SEC: u64 = 10;
const RETRY_MAX_INTERVAL_SEC: u64 = 60;
const IDLE_THRESHOLD_MS: u64 = 3000;

// ── 应用状态 ──────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppState {
    Idle,
    Monitoring,
    Countdown,
    Locking,
    AutoPaused,
    BleTurnedOff,
}

// ── 应用核心状态（使用 Arc<Mutex> 以支持跨线程访问）────────────────────────
struct AppCore {
    config: Config,
    app_state: AppState,
    paused: bool,
    bt_monitor: Option<BluetoothMonitor>,
    poller: Option<SmartPoller>,
    tray: TrayIcon,
    // 连接状态
    conn_status: String,
    conn_rssi: i32,
    ever_connected: bool,
    lock_abandoned: bool,
    // 自动连接
    auto_connecting: bool,
    auto_connect_attempts: i32,
    connecting: bool,
    // 倒计时
    countdown_started: bool,
    countdown_expired: bool,
    countdown_remaining: i32,
    // 重连
    retry_pending: bool,
    retry_round_count: i32,
    // 事件通道
    event_tx: Option<mpsc::Sender<BtEvent>>,
    event_rx: Option<mpsc::Receiver<BtEvent>>,
}

type SharedCore = Arc<Mutex<AppCore>>;

fn new_core(config: Config) -> SharedCore {
    let (tx, rx) = mpsc::channel();
    Arc::new(Mutex::new(AppCore {
        config,
        app_state: AppState::Idle,
        paused: false,
        bt_monitor: None,
        poller: None,
        tray: TrayIcon::new(),
        conn_status: "disconnected".into(),
        conn_rssi: -100,
        ever_connected: false,
        lock_abandoned: false,
        auto_connecting: false,
        auto_connect_attempts: 0,
        connecting: false,
        countdown_started: false,
        countdown_expired: false,
        countdown_remaining: 0,
        retry_pending: false,
        retry_round_count: 0,
        event_tx: Some(tx),
        event_rx: Some(rx),
    }))
}

// ── AppCore 辅助方法 ─────────────────────────────────────────────────────

impl AppCore {
    fn set_state(&mut self, new_state: AppState) {
        let old = self.app_state;
        self.app_state = new_state;
        if old != new_state {
            log::info!("状态: {old:?} → {new_state:?}");
        }
    }

    fn set_conn(&mut self, status: &str, rssi: i32) {
        self.conn_status = status.to_string();
        // 断连时清零RSSI，避免UI显示过时信号值
        if status == "disconnected" || status == "ble_off" {
            self.conn_rssi = -100;
        } else if rssi > -90 {
            self.conn_rssi = rssi;
        }
    }

    fn cancel_countdown(&mut self) {
        self.countdown_remaining = 0;
        self.countdown_started = false;
        self.countdown_expired = false;
        if self.app_state == AppState::Countdown {
            self.set_state(AppState::Monitoring);
        }
    }

    fn start_countdown(&mut self) {
        if matches!(
            self.app_state,
            AppState::Countdown | AppState::Locking | AppState::AutoPaused
        ) {
            return;
        }
        if self.countdown_started {
            return;
        }
        if !self.config.auto_lock_enabled {
            log::debug!("自动锁屏已关闭，跳过锁屏倒计时");
            self.set_state(AppState::Monitoring);
            return;
        }
        log::info!("启动锁屏倒计时: {}秒", self.config.lock_delay_sec);
        self.countdown_started = true;
        self.countdown_expired = false;
        self.set_state(AppState::Countdown);
        self.countdown_remaining = self.config.lock_delay_sec;
        let rssi = self
            .bt_monitor
            .as_ref()
            .map(|m| m.current_rssi)
            .unwrap_or(-100);
        self.set_conn("fading", rssi);
    }

    fn stop_auto_connect(&mut self) {
        self.auto_connecting = false;
        self.auto_connect_attempts = 0;
        self.lock_abandoned = false;
        self.retry_pending = false;
        self.retry_round_count = 0;
        self.connecting = false;
    }

    fn start_auto_connect(&mut self) {
        if self.config.phone_mac.is_empty() || self.bt_monitor.is_none() {
            return;
        }
        if self
            .bt_monitor
            .as_ref()
            .map(|m| m.connected)
            .unwrap_or(false)
        {
            return;
        }
        self.auto_connecting = true;
        self.auto_connect_attempts = 0;
        self.set_conn("reconnecting", -100);
    }
}

// ── 主函数 ──────────────────────────────────────────────────────────────
fn main() {
    // 强制 GTK4 使用 Adwaita 深色主题（必须在 Application 创建前设置）
    std::env::set_var("GTK_THEME", "Adwaita:dark");
    // 当前环境无法访问 GPU/EGL 时，使用 Cairo 软件渲染避免 GTK4 在首次绘制阶段崩溃。
    std::env::set_var("GSK_RENDERER", "cairo");

    log_capture::init().expect("日志初始化失败");

    let mut cfg = Config::load();
    let args: Vec<String> = std::env::args().collect();
    let non_unique_instance = args.iter().any(|arg| arg == "--new-instance");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mac" if i + 1 < args.len() => {
                cfg.phone_mac = args[i + 1].clone();
                i += 2;
            }
            "--delay" if i + 1 < args.len() => {
                if let Ok(v) = args[i + 1].parse() {
                    cfg.lock_delay_sec = v;
                }
                i += 2;
            }
            "--threshold" if i + 1 < args.len() => {
                if let Ok(v) = args[i + 1].parse() {
                    cfg.rssi_threshold = v;
                }
                i += 2;
            }
            "-v" | "--verbose" => {
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    cfg.save();
    log::info!(
        "配置加载完成: phone_mac={}, auto_lock={}, auto_connect={}, threshold={}dBm, lock_delay={}s",
        cfg.phone_mac, cfg.auto_lock_enabled, cfg.auto_connect_device,
        cfg.rssi_threshold, cfg.lock_delay_sec
    );
    if cfg.language.is_empty() {
        cfg.language = detect_language();
    }
    set_lang(&cfg.language);

    let app = Application::builder()
        .application_id("com.bt-proximity-lock")
        .build();

    let core: SharedCore = new_core(cfg);

    app.connect_activate(move |application| {
        ui::apply_css();
        let phone_name = {
            let c = core.lock().unwrap();
            c.config.phone_name.clone()
        };
        let win = ApplicationWindow::builder()
            .application(application)
            .title(&format!(
                "{} - {}",
                tr("title"),
                if phone_name.is_empty() {
                    "--"
                } else {
                    &phone_name
                }
            ))
            .default_width({
                let c = core.lock().unwrap();
                c.config.window_width
            })
            .default_height({
                let c = core.lock().unwrap();
                c.config.window_height
            })
            .build();

        // ── 构建 UI ──
        let main_box = GtkBox::new(gtk::Orientation::Vertical, 0);
        main_box.set_vexpand(true);
        main_box.set_hexpand(true);
        // 顶栏
        let header = GtkBox::new(gtk::Orientation::Vertical, 0);
        header.set_margin_top(20);
        header.set_margin_start(20);
        header.set_margin_end(20);
        header.set_margin_bottom(5);
        let title_label = gtk::Label::new(Some(&tr("title")));
        title_label.add_css_class("title-label");
        title_label.set_halign(gtk::Align::Start);
        header.append(&title_label);
        let subtitle_label = gtk::Label::new(Some(&tr("subtitle")));
        subtitle_label.add_css_class("subtitle-label");
        subtitle_label.set_halign(gtk::Align::Start);
        header.append(&subtitle_label);
        main_box.append(&header);

        // ── 设备选择页 ──
        let pick_widgets = ui::build_pick_page();
        pick_widgets.container.set_vexpand(true);
        let page_container = GtkBox::new(gtk::Orientation::Vertical, 0);
        page_container.set_hexpand(true);
        page_container.set_vexpand(true);
        page_container.set_visible(false);

        // ── 监控页（带回调）──
        // 使用 RefCell 延迟引用 mw（因为 mw 在 build_monitor_page 之后才创建）
        let mw_holder: Rc<RefCell<Option<Arc<Mutex<ui::MonitorWidgets>>>>> =
            Rc::new(RefCell::new(None));
        let mw_holder2 = mw_holder.clone();
        let mw: Arc<Mutex<ui::MonitorWidgets>> = Arc::new(Mutex::new(ui::build_monitor_page(
            {
                let c = core.clone();
                let w = win.clone();
                let mh = mw_holder2;
                move || {
                    if let Some(mw_ref) = mh.borrow().as_ref() {
                        on_toggle_monitor(&c, &w, mw_ref);
                    }
                }
            },
            {
                let c = core.clone();
                let dl = pick_widgets.device_list.clone();
                let win = win.clone();
                let pc = page_container.clone();
                let pk = pick_widgets.container.clone();
                move || {
                    stop_monitoring(&c);
                    pc.set_visible(false);
                    pk.set_visible(true);
                    let devices = scanner::scan_connected_devices();
                    log::info!("刷新设备列表: {} 个设备", devices.len());
                    let c2 = c.clone();
                    let w2 = win.clone();
                    let pc2 = pc.clone();
                    let pk2 = pk.clone();
                    let on_select: Rc<RefCell<Box<dyn Fn(&scanner::DeviceInfo)>>> =
                        Rc::new(RefCell::new(Box::new(move |dev| {
                            select_device_from_pick(&c2, &w2, &pc2, &pk2, dev);
                        })));
                    ui::update_device_list(&dl, &devices, on_select);
                }
            },
        )));

        {
            let c = core.clone();
            let dl = pick_widgets.device_list.clone();
            let win = win.clone();
            let pc = page_container.clone();
            let pk = pick_widgets.container.clone();
            pick_widgets.btn_refresh.connect_clicked(move |_| {
                refresh_device_list_ui(&dl, &win, &c, &pc, &pk);
            });
        }

        let mw_unlocked = mw.lock().unwrap();
        page_container.append(&mw_unlocked.container);
        drop(mw_unlocked);

        // 将 mw 放入 holder，使 toggle 回调可以访问
        *mw_holder.borrow_mut() = Some(mw.clone());

        main_box.append(&pick_widgets.container);
        main_box.append(&page_container);
        win.set_child(Some(&main_box));

        // 初始化显示 + 同步设置
        {
            let c = core.lock().unwrap();
            if c.config.phone_mac.is_empty() {
                drop(c);
                pick_widgets.container.set_visible(true);
                page_container.set_visible(false);
                let dl = pick_widgets.device_list.clone();
                let w2 = win.clone();
                let c2 = core.clone();
                let pc2 = page_container.clone();
                let pk2 = pick_widgets.container.clone();
                refresh_device_list_ui(&dl, &w2, &c2, &pc2, &pk2);
            } else {
                drop(c);
                pick_widgets.container.set_visible(false);
                page_container.set_visible(true);
                let mw_unlocked = mw.lock().unwrap();
                mw_unlocked.device_name.set_text(&core.lock().unwrap().config.phone_name);
                mw_unlocked.device_mac.set_text(&core.lock().unwrap().config.phone_mac);
                // 同步设置值到 UI
                let cfg = core.lock().unwrap().config.clone();
                ui::sync_settings_from_config(&mw_unlocked, &cfg);
                drop(mw_unlocked);
            }
        }

        // ── 设置控件回调绑定 ──
        {
            let c = core.clone();
            let mw2 = mw.clone();
            let mw3 = mw.clone();
            // 锁屏延迟滑块
            {
                let c2 = c.clone();
                let slider = { mw2.lock().unwrap().slider_lock_delay.clone() };
                slider.connect_value_changed(move |s| {
                    let v = s.value() as i32;
                    c2.lock().unwrap().config.lock_delay_sec = v;
                    c2.lock().unwrap().config.save();
                });
            }
            // 信号阈值滑块
            {
                let c2 = c.clone();
                let slider = { mw2.lock().unwrap().slider_rssi_threshold.clone() };
                slider.connect_value_changed(move |s| {
                    let v = s.value() as i32;
                    c2.lock().unwrap().config.rssi_threshold = v;
                    c2.lock().unwrap().config.save();
                });
            }
            // 轮询间隔滑块
            {
                let c2 = c.clone();
                let slider = { mw2.lock().unwrap().slider_scan_interval.clone() };
                slider.connect_value_changed(move |s| {
                    let v = s.value() as i32;
                    c2.lock().unwrap().config.scan_interval_sec = v;
                    c2.lock().unwrap().config.save();
                });
            }
            // 自动锁屏开关
            {
                let c2 = c.clone();
                let switch = { mw2.lock().unwrap().switch_auto_lock.clone() };
                switch.connect_active_notify(move |s| {
                    c2.lock().unwrap().config.auto_lock_enabled = s.is_active();
                    c2.lock().unwrap().config.save();
                });
            }
            // 自动解锁开关
            {
                let c2 = c.clone();
                let switch = { mw2.lock().unwrap().switch_auto_unlock.clone() };
                switch.connect_active_notify(move |s| {
                    c2.lock().unwrap().config.auto_unlock_on_reconnect = s.is_active();
                    c2.lock().unwrap().config.save();
                });
            }
            // 自动重连开关
            {
                let c2 = c.clone();
                let switch = { mw2.lock().unwrap().switch_auto_connect.clone() };
                switch.connect_active_notify(move |s| {
                    c2.lock().unwrap().config.auto_connect_device = s.is_active();
                    c2.lock().unwrap().config.save();
                });
            }
            // 锁屏后恢复监控开关
            {
                let c2 = c.clone();
                let switch = { mw2.lock().unwrap().switch_auto_resume.clone() };
                switch.connect_active_notify(move |s| {
                    c2.lock().unwrap().config.auto_resume_on_reconnect = s.is_active();
                    c2.lock().unwrap().config.save();
                });
            }
            // 断开蓝牙外设开关
            {
                let c2 = c.clone();
                let switch = { mw2.lock().unwrap().switch_disconnect_peripherals.clone() };
                switch.connect_active_notify(move |s| {
                    c2.lock().unwrap().config.disconnect_peripherals = s.is_active();
                    c2.lock().unwrap().config.save();
                });
            }
            // 开机自启开关
            {
                let c2 = c.clone();
                let switch = { mw2.lock().unwrap().switch_auto_start.clone() };
                switch.connect_active_notify(move |s| {
                    let enabled = s.is_active();
                    c2.lock().unwrap().config.auto_start_on_boot = enabled;
                    c2.lock().unwrap().config.save();
                    if enabled {
                        crate::autostart::enable_autostart();
                    } else {
                        crate::autostart::disable_autostart();
                    }
                });
            }
            // 恢复默认按钮
            {
                let c2 = c.clone();
                let mw_ref = mw3.clone();
                let btn = { mw3.lock().unwrap().btn_restore_defaults.clone() };
                btn.connect_clicked(move |_| {
                    let default_cfg = Config::default();
                    {
                        let mut c = c2.lock().unwrap();
                        c.config.lock_delay_sec = default_cfg.lock_delay_sec;
                        c.config.rssi_threshold = default_cfg.rssi_threshold;
                        c.config.scan_interval_sec = default_cfg.scan_interval_sec;
                        c.config.auto_lock_enabled = default_cfg.auto_lock_enabled;
                        c.config.auto_unlock_on_reconnect = default_cfg.auto_unlock_on_reconnect;
                        c.config.auto_connect_device = default_cfg.auto_connect_device;
                        c.config.auto_resume_on_reconnect = default_cfg.auto_resume_on_reconnect;
                        c.config.disconnect_peripherals = default_cfg.disconnect_peripherals;
                        c.config.auto_start_on_boot = default_cfg.auto_start_on_boot;
                        c.config.save();
                    }
                    // 禁用自启
                    crate::autostart::disable_autostart();
                    let cfg = c2.lock().unwrap().config.clone();
                    let mw_ui = mw_ref.lock().unwrap();
                    ui::sync_settings_from_config(&mw_ui, &cfg);
                    log::info!("设置已恢复默认值");
                });
            }
            // 诊断按钮（日志 + RSSI 曲线）
            {
                let btn = { mw3.lock().unwrap().btn_log.clone() };
                btn.connect_clicked(move |_| {
                    let logs = crate::log_capture::get_logs();
                    let rssi_history = crate::log_capture::get_rssi_history();
                    let curve = crate::log_capture::render_rssi_curve(&rssi_history, 80, 12);
                    ui::show_log_dialog(&logs, &curve);
                });
            }
        }

        // ── 定时器 ──
        // 蓝牙轮询（每3秒）
        {
            let c = core.clone();
            let mw2 = mw.clone();
            glib::timeout_add_seconds_local(3, move || {
                bt_poll_tick(&c, &mw2);
                glib::ControlFlow::Continue
            });
        }
        // 事件处理（每50ms）
        {
            let c = core.clone();
            let mw_clone = mw.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                process_bt_events(&c, &mw_clone);
                glib::ControlFlow::Continue
            });
        }
        // 倒计时（每1秒）
        {
            let c = core.clone();
            let mw2 = mw.clone();
            glib::timeout_add_seconds_local(1, move || {
                countdown_tick(&c, &mw2);
                glib::ControlFlow::Continue
            });
        }
        // 启动托盘
        {
            let mut c = core.lock().unwrap();
            c.tray.start();
        }

        win.present();
    });

    // GTK4 的 Application 会自动处理 SIGTERM 信号
    app.run();
}

// ── 页面切换 ──────────────────────────────────────────────────────────────

// ── 设备选择 ──────────────────────────────────────────────────────────────

// ── 监控控制 ──────────────────────────────────────────────────────────────

fn start_monitoring(core: &SharedCore) {
    let mut c = core.lock().unwrap();
    if c.config.phone_mac.is_empty() {
        return;
    }
    log::info!(
        "启动监控: device={}, auto_lock={}, auto_connect={}",
        c.config.phone_mac,
        c.config.auto_lock_enabled,
        c.config.auto_connect_device
    );
    let mut bt = BluetoothMonitor::new(&c.config.phone_mac, c.config.rssi_threshold);
    if let Some(tx) = c.event_tx.take() {
        bt.set_event_sender(tx);
    }
    if bt.setup() {
        log::info!("蓝牙监控初始化成功");
    } else {
        log::warn!("蓝牙监控初始化失败，等待适配器就绪");
    }
    c.bt_monitor = Some(bt);
    let mut poller = SmartPoller::new(c.config.rssi_threshold, c.config.scan_interval_sec);
    poller.start();
    c.poller = Some(poller);
    c.set_state(AppState::Monitoring);
}

fn stop_monitoring(core: &SharedCore) {
    log::info!("停止监控");
    let mut c = core.lock().unwrap();
    c.ever_connected = false;
    c.cancel_countdown();
    c.stop_auto_connect();
    if let Some(mut p) = c.poller.take() {
        p.stop();
    }
    c.bt_monitor.take();
    c.set_state(AppState::Idle);
    c.set_conn("disconnected", -100);
}

fn on_toggle_monitor(
    core: &SharedCore,
    _win: &ApplicationWindow,
    mw: &Arc<Mutex<ui::MonitorWidgets>>,
) {
    let state = core.lock().unwrap().app_state;
    match state {
        AppState::Idle => start_monitoring(core),
        AppState::AutoPaused => {
            let mut c = core.lock().unwrap();
            c.ever_connected = true;
            c.lock_abandoned = false;
            c.set_state(AppState::Monitoring);
        }
        _ => stop_monitoring(core),
    }
    // 立即更新UI，给用户即时反馈
    let c = core.lock().unwrap();
    update_ui(&c, mw);
}

fn refresh_device_list_ui(
    device_list: &gtk::Box,
    win: &ApplicationWindow,
    core: &SharedCore,
    page_container: &GtkBox,
    pick_container: &GtkBox,
) {
    let devices = scanner::scan_connected_devices();
    log::info!("刷新设备列表: {} 个设备", devices.len());
    let c = core.clone();
    let pc = page_container.clone();
    let pk = pick_container.clone();
    let w = win.clone();
    let on_select: Rc<RefCell<Box<dyn Fn(&scanner::DeviceInfo)>>> =
        Rc::new(RefCell::new(Box::new(move |dev| {
            select_device_from_pick(&c, &w, &pc, &pk, dev);
        })));
    ui::update_device_list(device_list, &devices, on_select);
}

fn select_device_from_pick(
    core: &SharedCore,
    win: &ApplicationWindow,
    page_container: &GtkBox,
    pick_container: &GtkBox,
    dev: &scanner::DeviceInfo,
) {
    log::info!("选择设备: {} ({})", dev.name, dev.mac);
    {
        let mut c = core.lock().unwrap();
        c.config.phone_mac = dev.mac.clone();
        c.config.phone_name = dev.name.clone();
        c.config.save();
    }
    pick_container.set_visible(false);
    page_container.set_visible(true);
    win.set_title(Some(&format!("{} - {}", tr("title"), &dev.name)));
    // 更新监控页设备信息
    if let Some(child) = page_container.first_child() {
        if let Ok(monitor_box) = child.downcast::<GtkBox>() {
            update_monitor_device_info(&monitor_box, &dev.name, &dev.mac);
        }
    }
    start_monitoring(core);
}

/// 更新监控页设备信息标签（递归搜索嵌套的 GtkBox）
fn update_monitor_device_info(box_widget: &GtkBox, name: &str, mac: &str) {
    let mut child = box_widget.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        if let Ok(label) = c.clone().downcast::<gtk::Label>() {
            let text = label.text().to_string();
            if text == "--" || text.is_empty() {
                if label.has_css_class("card-value") {
                    label.set_text(name);
                } else if label.has_css_class("card-mac") {
                    label.set_text(mac);
                }
            }
        } else if let Ok(inner_box) = c.clone().downcast::<GtkBox>() {
            update_monitor_device_info(&inner_box, name, mac);
        }
        child = next;
    }
}

// ── 蓝牙事件处理 ──────────────────────────────────────────────────────────

fn process_bt_events(core: &SharedCore, mw: &Arc<Mutex<ui::MonitorWidgets>>) {
    let events: Vec<BtEvent> = {
        let c = core.lock().unwrap();
        if let Some(rx) = &c.event_rx {
            rx.try_iter().collect()
        } else {
            return;
        }
    };
    for event in events {
        handle_bt_event(core, event, mw);
    }
}

fn handle_bt_event(core: &SharedCore, event: BtEvent, mw: &Arc<Mutex<ui::MonitorWidgets>>) {
    let mut c = core.lock().unwrap();
    if c.paused {
        return;
    }
    match event {
        BtEvent::Connected => {
            c.ever_connected = true;
            if c.app_state == AppState::Monitoring {
                return;
            }
            let rssi = c
                .bt_monitor
                .as_ref()
                .map(|m| m.current_rssi)
                .unwrap_or(-100);
            log::info!("设备已连接: rssi={rssi}dBm, 状态={:?}", c.app_state);
            c.stop_auto_connect();
            c.cancel_countdown();
            c.set_state(AppState::Monitoring);
            c.set_conn("connected", rssi);
            if c.config.auto_unlock_on_reconnect {
                locker::unlock_session();
            }
            // 确保UI更新
            update_ui(&c, &mw);
        }
        BtEvent::Disconnected => {
            if c.paused || c.app_state == AppState::Locking {
                return;
            }
            if c.app_state == AppState::AutoPaused {
                if c.config.auto_connect_device && !c.auto_connecting && !c.retry_pending {
                    c.start_auto_connect();
                }
                return;
            }
            if !matches!(c.app_state, AppState::Monitoring | AppState::Countdown) {
                return;
            }
            if c.retry_pending {
                return;
            }
            if c.config.auto_connect_device && !c.auto_connecting {
                c.start_auto_connect();
            } else if !c.config.auto_connect_device {
                judge_after_unreachable(&mut c, core);
            }
            // 确保UI更新
            update_ui(&c, &mw);
        }
        BtEvent::RssiChanged(rssi) => {
            if c.paused || c.bt_monitor.as_ref().map(|m| !m.connected).unwrap_or(true) {
                return;
            }
            if !c
                .bt_monitor
                .as_ref()
                .map(|m| m.rssi_supported)
                .unwrap_or(true)
            {
                return;
            }
            if matches!(
                c.app_state,
                AppState::Locking | AppState::Idle | AppState::AutoPaused
            ) {
                return;
            }
            let status = c.conn_status.clone();
            c.set_conn(&status, rssi);
            if c.app_state == AppState::Countdown {
                if rssi >= c.config.rssi_threshold {
                    log::info!(
                        "信号恢复: {rssi}dBm >= 阈值{}dBm，取消锁屏",
                        c.config.rssi_threshold
                    );
                    c.cancel_countdown();
                    c.set_state(AppState::Monitoring);
                }
                return;
            }
            if rssi < c.config.rssi_threshold {
                log::warn!("信号低于阈值: {rssi}dBm < {}dBm", c.config.rssi_threshold);
                c.start_countdown();
            }
        }
        BtEvent::AdapterPoweredOff => {
            log::info!("蓝牙适配器已关闭");
            c.cancel_countdown();
            c.set_state(AppState::BleTurnedOff);
            c.set_conn("ble_off", -100);
            // 确保UI更新
            update_ui(&c, &mw);
        }
        BtEvent::AdapterPoweredOn => {
            log::info!("蓝牙适配器已开启");
            if c.app_state == AppState::BleTurnedOff {
                c.set_state(AppState::Monitoring);
                if c.bt_monitor.as_ref().map(|m| m.connected).unwrap_or(false) {
                    let rssi = c
                        .bt_monitor
                        .as_ref()
                        .map(|m| m.current_rssi)
                        .unwrap_or(-100);
                    c.set_conn("connected", rssi);
                } else if c.config.auto_connect_device && !c.config.phone_mac.is_empty() {
                    c.set_conn("reconnecting", -100);
                    c.start_auto_connect();
                } else {
                    c.set_conn("disconnected", -100);
                }
            }
            // 确保UI更新
            update_ui(&c, &mw);
        }
    }
}

// ── 轮询 ──────────────────────────────────────────────────────────────────

fn bt_poll_tick(core: &SharedCore, mw: &Arc<Mutex<ui::MonitorWidgets>>) {
    let mut c = core.lock().unwrap();
    if matches!(c.app_state, AppState::Idle | AppState::Locking) {
        return;
    }
    if let Some(bt) = &mut c.bt_monitor {
        bt.poll_status();
        let connected = bt.connected;
        let rssi = bt.filtered_rssi;
        let rssi_supported = bt.rssi_supported;
        // 记录 RSSI 到历史
        if rssi > -90 {
            crate::log_capture::push_rssi(rssi);
        }
        if let Some(poller) = &mut c.poller {
            let (_next_ms, trigger_disconnect) = poller.tick(connected, rssi, rssi_supported);
            if trigger_disconnect {
                drop(c);
                handle_bt_event(core, BtEvent::Disconnected, mw);
                return;
            }
        }
        if connected {
            c.ever_connected = true;
        }
        // 直接同步连接状态到 UI，不依赖事件通道
        if connected {
            c.set_conn("connected", rssi);
        } else if c.app_state == AppState::Monitoring {
            c.set_conn("disconnected", rssi);
        }
    }
    update_ui(&c, mw);
}

// ── 倒计时 ────────────────────────────────────────────────────────────────

fn countdown_tick(core: &SharedCore, mw: &Arc<Mutex<ui::MonitorWidgets>>) {
    let mut c = core.lock().unwrap();
    if c.app_state != AppState::Countdown || c.countdown_remaining <= 0 {
        return;
    }
    let idle_ms = locker::get_idle_ms();
    if idle_ms < IDLE_THRESHOLD_MS {
        log::info!("检测到用户活动，取消锁屏倒计时");
        c.cancel_countdown();
        c.lock_abandoned = true;
        return;
    }
    c.countdown_remaining -= 1;
    if c.countdown_remaining > 0 {
        let mw = mw.lock().unwrap();
        mw.status_label.set_text(&tr_fmt(
            "status_countdown",
            &[("n", &c.countdown_remaining.to_string())],
        ));
    } else {
        c.countdown_expired = true;
        drop(c);
        do_lock_screen(core, mw);
    }
}

fn do_lock_screen(core: &SharedCore, mw: &Arc<Mutex<ui::MonitorWidgets>>) {
    let mut c = core.lock().unwrap();
    if matches!(c.app_state, AppState::Locking | AppState::AutoPaused) {
        return;
    }
    let idle_ms = locker::get_idle_ms();
    if idle_ms < IDLE_THRESHOLD_MS {
        log::info!("锁屏前检测到用户活动，取消锁屏");
        c.set_state(AppState::Monitoring);
        c.countdown_started = false;
        c.lock_abandoned = true;
        return;
    }
    log::info!("执行锁屏");
    c.set_state(AppState::Locking);
    c.countdown_started = false;
    let mw_ui = mw.lock().unwrap();
    mw_ui.status_label.set_text(&tr("status_locking"));
    let locked = locker::lock_screen();
    if locked {
        let mw_ui = mw.lock().unwrap();
        mw_ui.status_label.set_text(&tr("status_locked"));
        if c.config.auto_resume_on_reconnect {
            c.set_state(AppState::AutoPaused);
            let mw_ui = mw.lock().unwrap();
            mw_ui.status_label.set_text(&tr("status_paused_waiting"));
        } else {
            drop(c);
            stop_monitoring(core);
            return;
        }
    } else {
        let mw_ui = mw.lock().unwrap();
        mw_ui.status_label.set_text(&tr("status_lock_failed"));
        c.set_state(AppState::Monitoring);
    }
}

// ── 自动连接 ──────────────────────────────────────────────────────────────

fn start_connect_attempt(core: &SharedCore) {
    let mut c = core.lock().unwrap();
    if c.connecting || c.bt_monitor.is_none() {
        return;
    }
    if c.auto_connect_attempts >= MAX_CONNECT_ATTEMPTS {
        return;
    }
    c.connecting = true;
    c.auto_connect_attempts += 1;
    if let Some(bt) = &c.bt_monitor {
        let (le_proc, br_proc) = bt.start_connect_process(CONNECT_TIMEOUT_SEC);
        let core_clone = core.clone();
        glib::spawn_future(async move {
            poll_connect_result(core_clone, le_proc, br_proc).await;
        });
    }
}

async fn poll_connect_result(
    core: SharedCore,
    mut le_proc: Option<std::process::Child>,
    mut br_proc: Option<std::process::Child>,
) {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(CONNECT_TIMEOUT_SEC + 10);
    loop {
        if start.elapsed() > timeout {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        let connected = {
            let mut c = core.lock().unwrap();
            c.bt_monitor
                .as_mut()
                .map(|bt| bt.check_connected_now())
                .unwrap_or(false)
        };
        if connected {
            log::info!("连接成功（检测到已连接）");
            kill_procs(&mut le_proc, &mut br_proc);
            {
                core.lock().unwrap().connecting = false;
            }
            on_reconnect_success(&core);
            return;
        }
        let le_done = le_proc
            .as_mut()
            .map(|p| p.try_wait().ok().flatten().is_some())
            .unwrap_or(true);
        let br_done = br_proc
            .as_mut()
            .map(|p| p.try_wait().ok().flatten().is_some())
            .unwrap_or(true);
        if le_done && br_done {
            log::info!(
                "第 {} 次连接失败",
                core.lock().unwrap().auto_connect_attempts
            );
            kill_procs(&mut le_proc, &mut br_proc);
            {
                core.lock().unwrap().connecting = false;
            }
            after_connect_failed(&core);
            return;
        }
    }
    kill_procs(&mut le_proc, &mut br_proc);
    {
        core.lock().unwrap().connecting = false;
    }
    after_connect_failed(&core);
}

fn kill_procs(le: &mut Option<std::process::Child>, br: &mut Option<std::process::Child>) {
    for p in [le, br].iter_mut() {
        if let Some(mut proc) = p.take() {
            let _ = proc.kill();
        }
    }
}

fn on_reconnect_success(core: &SharedCore) {
    let mut c = core.lock().unwrap();
    log::info!("重连成功: 状态={:?}", c.app_state);
    c.ever_connected = true;
    c.lock_abandoned = false;
    c.retry_round_count = 0;
    c.retry_pending = false;
    if c.app_state == AppState::AutoPaused {
        c.set_state(AppState::Monitoring);
        let rssi = c
            .bt_monitor
            .as_ref()
            .map(|m| m.filtered_rssi)
            .unwrap_or(-100);
        c.set_conn("connected", rssi);
        if c.config.auto_unlock_on_reconnect {
            locker::unlock_session();
        }
        return;
    }
    c.auto_connecting = false;
    c.auto_connect_attempts = 0;
    c.cancel_countdown();
    c.set_state(AppState::Monitoring);
    let rssi = c
        .bt_monitor
        .as_ref()
        .map(|m| m.filtered_rssi)
        .unwrap_or(-100);
    c.set_conn("connected", rssi);
    if c.config.auto_unlock_on_reconnect {
        locker::unlock_session();
    }
}

fn after_connect_failed(core: &SharedCore) {
    let attempts = core.lock().unwrap().auto_connect_attempts;
    if attempts < MAX_CONNECT_ATTEMPTS {
        let core_clone = core.clone();
        glib::timeout_add_seconds_local(CONNECT_ATTEMPT_INTERVAL_SEC, move || {
            start_connect_attempt(&core_clone);
            glib::ControlFlow::Break
        });
        return;
    }
    log::info!("连续 {MAX_CONNECT_ATTEMPTS} 次连接失败，判定手机不在周围");
    let mut c = core.lock().unwrap();
    judge_after_unreachable(&mut c, core);
}

fn judge_after_unreachable(c: &mut AppCore, core: &SharedCore) {
    if c.app_state == AppState::AutoPaused {
        schedule_retry_round(core);
        return;
    }
    if c.lock_abandoned {
        // 用户活跃期间跳过锁屏，但仍需安排重试轮次
        // 当用户停止活动后，清除lock_abandoned允许重新锁屏
        let idle_ms = locker::get_idle_ms();
        if idle_ms >= IDLE_THRESHOLD_MS {
            log::info!("用户已停止活动，清除lock_abandoned标志，准备重新锁屏");
            c.lock_abandoned = false;
            c.start_countdown();
        } else {
            schedule_retry_round(core);
        }
        return;
    }
    let idle_ms = locker::get_idle_ms();
    if idle_ms >= IDLE_THRESHOLD_MS {
        c.start_countdown();
    } else {
        c.lock_abandoned = true;
        log::info!("电脑处于活跃状态，本轮不再锁屏，只持续尝试重连");
    }
    schedule_retry_round(core);
}

fn schedule_retry_round(core: &SharedCore) {
    let mut c = core.lock().unwrap();
    if !c.config.auto_connect_device || c.retry_pending {
        return;
    }
    c.auto_connecting = true;
    c.retry_pending = true;
    let delay = std::cmp::min(
        RETRY_BASE_INTERVAL_SEC * 2u64.pow(c.retry_round_count as u32),
        RETRY_MAX_INTERVAL_SEC,
    ) as u32;
    let round = c.retry_round_count;
    log::info!("将在 {delay}秒 后进行第 {round} 轮重连");
    drop(c);
    let core_clone = core.clone();
    glib::timeout_add_seconds_local(delay, move || {
        retry_round(&core_clone);
        glib::ControlFlow::Break
    });
}

fn retry_round(core: &SharedCore) {
    let mut c = core.lock().unwrap();
    c.retry_pending = false;
    if matches!(
        c.app_state,
        AppState::Locking | AppState::Idle | AppState::BleTurnedOff
    ) || c.paused
    {
        c.auto_connecting = false;
        c.retry_round_count = 0;
        return;
    }
    if c.bt_monitor.as_ref().map(|m| m.connected).unwrap_or(false) {
        c.auto_connecting = false;
        c.retry_round_count = 0;
        return;
    }
    if c.connecting {
        drop(c);
        schedule_retry_round(core);
        return;
    }
    c.retry_round_count += 1;
    c.auto_connect_attempts = 0;
    drop(c);
    start_connect_attempt(core);
}

// ── UI 刷新 ────────────────────────────────────────────────────────────────

fn update_ui(c: &AppCore, mw: &Arc<Mutex<ui::MonitorWidgets>>) {
    let conn = &c.conn_status;
    let rssi = c.conn_rssi;
    let mw = mw.lock().unwrap();
    ui::set_status_dot_color(&mw.status_dot, conn);
    ui::update_conn_status(&mw.conn_status, conn);
    if rssi > -90 {
        mw.rssi_text.set_text(&format!("{rssi} dBm"));
        ui::update_signal_bar(&mw.signal_bar, rssi);
    } else {
        mw.rssi_text.set_text("- dBm");
        mw.signal_bar.set_fraction(0.0);
    }
    let state_str = match c.app_state {
        AppState::Idle => "idle",
        AppState::AutoPaused => "auto_paused",
        _ => "active",
    };
    ui::update_toggle_button(&mw.btn_toggle, state_str);

    // 更新状态栏文本
    let status_text = match c.app_state {
        AppState::Idle => i18n::tr("status_monitoring_off"),
        AppState::Monitoring => {
            if c.ever_connected {
                i18n::tr("status_monitoring_on")
            } else {
                i18n::tr("status_scanning")
            }
        }
        AppState::Countdown => {
            i18n::tr_fmt("status_countdown", &[("n", &c.countdown_remaining.to_string())])
        }
        AppState::Locking => i18n::tr("status_locking"),
        AppState::AutoPaused => i18n::tr("status_paused_waiting"),
        AppState::BleTurnedOff => i18n::tr("status_ble_turned_off"),
    };
    mw.status_label.set_text(&status_text);
}
