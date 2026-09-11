/// UI 组件模块
/// GTK4 界面构建：设备选择页、监控页、设置区、状态栏、日志弹窗
use gtk::prelude::*;
use gtk::{self, Box as GtkBox, Button};
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::Config;
use crate::i18n::tr;
use crate::scanner::DeviceInfo;

// ── GTK4 CSS（深色主题 + 原版配色）───────────────────────────────────────
const CSS: &str = r#"
/* 强制深色配色方案 - GTK4 Adwaita 认这个 */
@define-color accent_color #3498db;
@define-color window_bg_color #1a1a2e;
@define-color window_fg_color #ecf0f1;
@define-color card_bg_color #16213e;
@define-color card_fg_color #ecf0f1;
@define-color headerbar_bg_color #0f0f23;
@define-color headerbar_fg_color #ecf0f1;
@define-color view_bg_color #1a1a2e;
@define-color view_fg_color #ecf0f1;
@define-color dialog_bg_color #16213e;
@define-color dialog_fg_color #ecf0f1;
@define-color popover_bg_color #16213e;
@define-color popover_fg_color #ecf0f1;

window {
    background-color: #1a1a2e;
}

.titlebar, headerbar {
    background-color: #0f0f23;
    color: #ecf0f1;
}

.title-label {
    font-weight: bold;
    font-size: 22px;
    color: #ecf0f1;
}

.subtitle-label {
    font-size: 13px;
    color: #95a5a6;
}

.card {
    background-color: #16213e;
    border-radius: 10px;
    border: 1px solid #0f3460;
    padding: 12px 14px;
    color: #ecf0f1;
}

.card-title {
    font-size: 12px;
    color: #95a5a6;
}

.card-value {
    font-weight: bold;
    font-size: 16px;
    color: #ecf0f1;
}

.card-mac {
    font-size: 12px;
    color: #7f8c8d;
    font-family: monospace;
}

.device-card {
    background-color: #16213e;
    border-radius: 10px;
    border: 1px solid #0f3460;
    padding: 12px 14px;
    margin: 4px 0;
    color: #ecf0f1;
}

.device-card:hover {
    background-color: #1a2744;
    border-color: #3498db;
}

.conn-status {
    font-weight: bold;
    font-size: 15px;
    color: #ecf0f1;
}

.status-bar {
    background-color: #0f3460;
    padding: 8px 14px;
    border-radius: 8px;
    border: 1px solid #1a1a4e;
    margin-top: 8px;
    color: #ecf0f1;
}

.empty-hint {
    font-size: 14px;
    color: #95a5a6;
    padding: 40px 0;
}

/* 强制所有文字为浅色 */
label {
    color: #ecf0f1;
}

/* 强制按钮深色主题 */
button {
    background-color: #2d3748;
    color: #ecf0f1;
    border: 1px solid #4a5568;
}

button:hover {
    background-color: #3d4a5c;
}

/* 强制滚动区域深色 */
scrollable, scrolledwindow {
    background-color: #1a1a2e;
}

/* 进度条 */
progressbar trough {
    background-color: #2d3748;
}
progressbar progress {
    background-color: #3498db;
}

/* 设置区域 */
.settings-row {
    padding: 4px 0;
}

.settings-row-label {
    font-size: 13px;
    color: #bdc3c7;
    min-width: 160px;
}

.settings-row-value {
    font-size: 13px;
    color: #62c7ff;
    font-weight: bold;
    min-width: 60px;
}

.settings-action-row {
    margin-top: 4px;
}

/* 滑块 */
scale trough {
    background-color: #101827;
    border: 1px solid #52627a;
    border-radius: 4px;
    min-height: 6px;
}

scale highlight {
    background-color: #38a9ff;
    border-radius: 4px;
    min-height: 6px;
}

scale slider {
    background-color: #ffffff;
    border: 2px solid #38a9ff;
    border-radius: 50%;
    min-width: 16px;
    min-height: 16px;
    margin: -5px;
}

/* 开关 */
switch {
    background-color: #2d3748;
    border-radius: 12px;
    min-width: 38px;
    min-height: 20px;
}

switch slider {
    background-color: #95a5a6;
    border-radius: 50%;
    min-width: 16px;
    min-height: 16px;
    margin: 2px;
}

switch:checked {
    background-color: #27ae60;
}

switch:checked slider {
    background-color: #ecf0f1;
}

/* 日志文本视图 */
.log-textview {
    background-color: #0d1117;
    color: #c9d1d9;
    font-size: 12px;
    padding: 8px;
}
"#;

/// 应用 CSS 主题
pub fn apply_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("无法获取默认显示器"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

// ── 构建设备选择页 ─────────────────────────────────────────────────────────

pub struct PickPageWidgets {
    pub container: GtkBox,
    pub device_list: GtkBox,
    pub btn_refresh: Button,
}

pub fn build_pick_page() -> PickPageWidgets {
    let container = GtkBox::new(gtk::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.set_margin_top(10);
    container.set_margin_start(15);
    container.set_margin_end(15);
    container.set_margin_bottom(10);

    let title = gtk::Label::new(Some(&tr("pick_title")));
    title.add_css_class("title-label");
    title.set_halign(gtk::Align::Start);
    container.append(&title);

    let hint = gtk::Label::new(Some(&tr("pick_hint")));
    hint.add_css_class("subtitle-label");
    hint.set_halign(gtk::Align::Start);
    container.append(&hint);

    // 设备列表（滚动区域）
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_margin_top(8);
    let device_list = GtkBox::new(gtk::Orientation::Vertical, 0);
    scroll.set_child(Some(&device_list));
    container.append(&scroll);

    // 刷新按钮
    let btn_box = GtkBox::new(gtk::Orientation::Horizontal, 0);
    btn_box.set_halign(gtk::Align::End);
    btn_box.set_margin_top(10);
    let btn_refresh = Button::with_label(&tr("btn_refresh"));
    btn_refresh.set_hexpand(false);
    btn_box.append(&btn_refresh);
    container.append(&btn_box);

    PickPageWidgets {
        container,
        device_list,
        btn_refresh,
    }
}

/// 更新设备列表
pub fn update_device_list(
    device_list: &GtkBox,
    devices: &[DeviceInfo],
    on_select: Rc<RefCell<Box<dyn Fn(&DeviceInfo)>>>,
) {
    // 清空旧设备
    while let Some(child) = device_list.first_child() {
        device_list.remove(&child);
    }

    if devices.is_empty() {
        let empty_label = gtk::Label::new(Some(&tr("pick_no_devices")));
        empty_label.add_css_class("empty-hint");
        empty_label.set_halign(gtk::Align::Center);
        device_list.append(&empty_label);
        return;
    }

    // 添加设备卡片
    for device in devices {
        let card = create_device_card(device, on_select.clone());
        device_list.append(&card);
    }
}

/// 创建设备卡片
pub fn create_device_card(
    device: &DeviceInfo,
    on_select: Rc<RefCell<Box<dyn Fn(&DeviceInfo)>>>,
) -> GtkBox {
    let card = GtkBox::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("device-card");

    // 第一行：设备名 + 连接状态
    let row1 = GtkBox::new(gtk::Orientation::Horizontal, 8);
    let name_label = gtk::Label::new(Some(&device.name));
    name_label.add_css_class("card-value");
    name_label.set_halign(gtk::Align::Start);
    name_label.set_hexpand(true);
    row1.append(&name_label);

    let status_text = if device.connected {
        "● 已连接"
    } else {
        "○ 未连接"
    };
    let status_label = gtk::Label::new(Some(status_text));
    status_label.set_halign(gtk::Align::End);
    row1.append(&status_label);
    card.append(&row1);

    // 第二行：MAC 地址
    let mac_label = gtk::Label::new(Some(&device.mac));
    mac_label.add_css_class("card-mac");
    mac_label.set_halign(gtk::Align::Start);
    card.append(&mac_label);

    let dev = device.clone();
    let gesture = gtk::GestureClick::new();
    gesture.connect_pressed(move |_, _, _, _| {
        on_select.borrow()(&dev);
    });
    card.add_controller(gesture);

    card
}

// ── 构建监控页 ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MonitorWidgets {
    pub container: GtkBox,
    pub device_name: gtk::Label,
    pub device_mac: gtk::Label,
    pub status_dot: gtk::Label,
    pub conn_status: gtk::Label,
    pub rssi_text: gtk::Label,
    pub signal_bar: gtk::ProgressBar,
    pub status_label: gtk::Label,
    pub btn_toggle: Button,
    pub btn_change: Button,
    // 设置 - 滑块
    pub slider_lock_delay: gtk::Scale,
    pub label_lock_delay_value: gtk::Label,
    pub slider_rssi_threshold: gtk::Scale,
    pub label_rssi_threshold_value: gtk::Label,
    pub slider_scan_interval: gtk::Scale,
    pub label_scan_interval_value: gtk::Label,
    // 设置 - 开关
    pub switch_auto_lock: gtk::Switch,
    pub switch_auto_unlock: gtk::Switch,
    pub switch_auto_connect: gtk::Switch,
    pub switch_auto_resume: gtk::Switch,
    pub switch_disconnect_peripherals: gtk::Switch,
    pub switch_auto_start: gtk::Switch,
    // 恢复默认
    pub btn_restore_defaults: Button,
    // 诊断
    pub btn_log: Button,
}

/// 辅助：构建滑块设置行（名称、滑块、默认值同一行）
fn build_slider_row(
    title: &str,
    min: f64,
    max: f64,
    initial: f64,
    unit: &str,
    slider_ref: &gtk::Scale,
    label_ref: &gtk::Label,
) -> GtkBox {
    let row = GtkBox::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("settings-row");
    row.set_valign(gtk::Align::Center);

    let lbl = gtk::Label::new(Some(title));
    lbl.add_css_class("settings-row-label");
    lbl.set_halign(gtk::Align::Start);
    row.append(&lbl);

    slider_ref.set_range(min, max);
    slider_ref.set_hexpand(true);
    slider_ref.set_draw_value(false);
    slider_ref.set_value(initial);
    slider_ref.set_digits(0);
    slider_ref.set_has_origin(false);
    row.append(slider_ref);

    label_ref.add_css_class("settings-row-value");
    label_ref.set_halign(gtk::Align::End);
    label_ref.set_width_chars(5);
    label_ref.set_text(&format!("{initial:.0}{unit}"));
    row.append(label_ref);

    let label_clone = label_ref.clone();
    let unit_s = unit.to_string();
    slider_ref.connect_value_changed(move |s| {
        let v = s.value() as i32;
        label_clone.set_text(&format!("{v}{unit_s}"));
    });
    row
}

/// 辅助：构建开关设置行
fn build_switch_row(title: &str, initial: bool, switch_ref: &gtk::Switch) -> GtkBox {
    let row = GtkBox::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("settings-row");
    row.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some(title));
    label.add_css_class("settings-row-label");
    label.set_halign(gtk::Align::Start);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&label);
    switch_ref.set_active(initial);
    switch_ref.set_halign(gtk::Align::End);
    row.append(switch_ref);
    row
}

/// 构建设置卡片（折叠式，点击标题展开/收起）
fn build_settings_card(mw: &MonitorWidgets) -> GtkBox {
    let card = GtkBox::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    card.set_margin_top(4);

    // 标题行（可点击折叠）
    let header = GtkBox::new(gtk::Orientation::Horizontal, 0);
    header.set_margin_top(8);
    header.set_margin_start(5);
    header.set_margin_bottom(4);
    let collapse_label = gtk::Label::new(Some("▾"));
    collapse_label.set_margin_end(6);
    header.append(&collapse_label);
    let title = gtk::Label::new(Some(&tr("label_settings")));
    title.add_css_class("card-title");
    title.set_halign(gtk::Align::Start);
    title.set_hexpand(true);
    header.append(&title);
    card.append(&header);

    // 可折叠内容区
    let content = GtkBox::new(gtk::Orientation::Vertical, 0);
    content.set_margin_start(5);
    content.set_margin_end(5);
    content.set_margin_bottom(4);

    // ── 滑块设置 ──
    content.append(&build_slider_row(
        &tr("settings_lock_delay"), 1.0, 60.0, 9.0, "s",
        &mw.slider_lock_delay, &mw.label_lock_delay_value,
    ));
    content.append(&build_slider_row(
        &tr("settings_rssi_threshold"), -90.0, -20.0, -50.0, "dBm",
        &mw.slider_rssi_threshold, &mw.label_rssi_threshold_value,
    ));
    content.append(&build_slider_row(
        &tr("settings_scan_interval"), 1.0, 30.0, 3.0, "s",
        &mw.slider_scan_interval, &mw.label_scan_interval_value,
    ));

    // ── 开关设置 ──
    content.append(&build_switch_row(&tr("settings_auto_lock"), true, &mw.switch_auto_lock));
    content.append(&build_switch_row(&tr("settings_auto_unlock"), false, &mw.switch_auto_unlock));
    content.append(&build_switch_row(&tr("settings_auto_reconnect"), true, &mw.switch_auto_connect));
    content.append(&build_switch_row(&tr("settings_auto_resume"), true, &mw.switch_auto_resume));
    content.append(&build_switch_row(&tr("settings_disconnect_peripherals"), false, &mw.switch_disconnect_peripherals));
    content.append(&build_switch_row(&tr("settings_auto_start"), false, &mw.switch_auto_start));

    {
        let btn_box = GtkBox::new(gtk::Orientation::Horizontal, 8);
        btn_box.add_css_class("settings-action-row");
        btn_box.set_halign(gtk::Align::End);
        btn_box.append(&mw.btn_log);
        btn_box.append(&mw.btn_restore_defaults);
        content.append(&btn_box);
    }

    // 点击标题折叠/展开
    let content_ref = content.clone();
    let collapse_ref = collapse_label.clone();
    let gesture = gtk::GestureClick::new();
    gesture.connect_pressed(move |_, _, _, _| {
        let visible = content_ref.is_visible();
        content_ref.set_visible(!visible);
        collapse_ref.set_text(if visible { "▸" } else { "▾" });
    });
    header.add_controller(gesture);

    card.append(&content);

    card
}

/// 构建监控页（包含设置区、诊断区、日志弹窗）
pub fn build_monitor_page(
    on_toggle: impl Fn() + 'static,
    on_change: impl Fn() + 'static,
) -> MonitorWidgets {
    let container = GtkBox::new(gtk::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.set_margin_top(5);
    container.set_margin_start(15);
    container.set_margin_end(15);

    // ── 设备信息卡片 ──
    let device_card = GtkBox::new(gtk::Orientation::Vertical, 0);
    device_card.add_css_class("card");
    let device_title = gtk::Label::new(Some(&tr("label_device")));
    device_title.add_css_class("card-title");
    device_title.set_halign(gtk::Align::Start);
    device_title.set_margin_top(8);
    device_title.set_margin_start(5);
    device_card.append(&device_title);

    let device_name = gtk::Label::new(Some("--"));
    device_name.add_css_class("card-value");
    device_name.set_halign(gtk::Align::Start);
    device_name.set_margin_start(5);
    device_card.append(&device_name);

    let device_mac = gtk::Label::new(Some("--"));
    device_mac.add_css_class("card-mac");
    device_mac.set_halign(gtk::Align::Start);
    device_mac.set_margin_start(5);
    device_mac.set_margin_bottom(8);
    device_card.append(&device_mac);
    container.append(&device_card);

    // ── 信号状态卡片 ──
    let signal_card = GtkBox::new(gtk::Orientation::Vertical, 0);
    signal_card.add_css_class("card");
    signal_card.set_margin_top(4);

    let signal_header = GtkBox::new(gtk::Orientation::Horizontal, 8);
    signal_header.set_margin_top(8);
    signal_header.set_margin_start(10);
    signal_header.set_margin_end(10);
    let status_dot = gtk::Label::new(Some("●"));
    status_dot.add_css_class("status-dot");
    signal_header.append(&status_dot);
    let conn_status = gtk::Label::new(Some(&tr("conn_disconnected")));
    conn_status.add_css_class("conn-status");
    conn_status.set_halign(gtk::Align::Start);
    conn_status.set_hexpand(true);
    signal_header.append(&conn_status);
    let rssi_text = gtk::Label::new(Some("- dBm"));
    rssi_text.add_css_class("card-mac");
    signal_header.append(&rssi_text);
    signal_card.append(&signal_header);

    let signal_row = GtkBox::new(gtk::Orientation::Horizontal, 8);
    signal_row.set_margin_bottom(8);
    signal_row.set_margin_start(10);
    signal_row.set_margin_end(10);
    let signal_title = gtk::Label::new(Some(&tr("label_signal")));
    signal_title.add_css_class("card-title");
    signal_row.append(&signal_title);
    let signal_bar = gtk::ProgressBar::new();
    signal_bar.set_hexpand(true);
    signal_bar.set_show_text(false);
    signal_row.append(&signal_bar);
    signal_card.append(&signal_row);
    container.append(&signal_card);

    // ── 按钮行（开始监控 + 更换设备）──
    let btn_box = GtkBox::new(gtk::Orientation::Horizontal, 8);
    btn_box.set_margin_top(10);
    btn_box.set_hexpand(true);

    let btn_toggle = Button::with_label(&tr("btn_start"));
    btn_toggle.set_hexpand(true);
    let on_toggle_cb = Rc::new(RefCell::new(on_toggle));
    let cb = on_toggle_cb.clone();
    btn_toggle.connect_clicked(move |_| {
        cb.borrow()();
    });
    btn_box.append(&btn_toggle);

    let btn_change = Button::with_label(&tr("btn_change"));
    let on_change_cb = Rc::new(RefCell::new(on_change));
    let cb = on_change_cb.clone();
    btn_change.connect_clicked(move |_| {
        cb.borrow()();
    });
    btn_box.append(&btn_change);

    // ── 状态栏 ──
    let status_bar = GtkBox::new(gtk::Orientation::Horizontal, 0);
    status_bar.add_css_class("status-bar");
    status_bar.set_margin_top(8);
    let status_label = gtk::Label::new(Some(&tr("status_ready")));
    status_label.set_halign(gtk::Align::Start);
    status_bar.append(&status_label);

    // ── 先创建 MonitorWidgets（设置控件需要引用）──
    let mw = MonitorWidgets {
        container: container.clone(),
        device_name: device_name.clone(),
        device_mac: device_mac.clone(),
        status_dot: status_dot.clone(),
        conn_status: conn_status.clone(),
        rssi_text: rssi_text.clone(),
        signal_bar: signal_bar.clone(),
        status_label: status_label.clone(),
        btn_toggle: btn_toggle.clone(),
        btn_change: btn_change.clone(),
        // 滑块
        slider_lock_delay: gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 60.0, 1.0),
        label_lock_delay_value: gtk::Label::new(None),
        slider_rssi_threshold: gtk::Scale::with_range(gtk::Orientation::Horizontal, -90.0, -20.0, 1.0),
        label_rssi_threshold_value: gtk::Label::new(None),
        slider_scan_interval: gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 30.0, 1.0),
        label_scan_interval_value: gtk::Label::new(None),
        // 开关
        switch_auto_lock: gtk::Switch::new(),
        switch_auto_unlock: gtk::Switch::new(),
        switch_auto_connect: gtk::Switch::new(),
        switch_auto_resume: gtk::Switch::new(),
        switch_disconnect_peripherals: gtk::Switch::new(),
        switch_auto_start: gtk::Switch::new(),
        // 按钮
        btn_restore_defaults: Button::with_label(&tr("btn_restore_defaults")),
        btn_log: Button::with_label(&tr("btn_test")),
    };

    // ── 设置卡片 ──
    let settings_card = build_settings_card(&mw);
    container.append(&settings_card);
    container.append(&btn_box);
    container.append(&status_bar);

    mw
}

/// 从 Config 同步设置值到 UI 控件
pub fn sync_settings_from_config(mw: &MonitorWidgets, cfg: &Config) {
    mw.slider_lock_delay.set_value(cfg.lock_delay_sec as f64);
    mw.label_lock_delay_value
        .set_text(&format!("{}s", cfg.lock_delay_sec));
    mw.slider_rssi_threshold.set_value(cfg.rssi_threshold as f64);
    mw.label_rssi_threshold_value
        .set_text(&format!("{}dBm", cfg.rssi_threshold));
    mw.slider_scan_interval.set_value(cfg.scan_interval_sec as f64);
    mw.label_scan_interval_value
        .set_text(&format!("{}s", cfg.scan_interval_sec));
    mw.switch_auto_lock.set_active(cfg.auto_lock_enabled);
    mw.switch_auto_unlock.set_active(cfg.auto_unlock_on_reconnect);
    mw.switch_auto_connect.set_active(cfg.auto_connect_device);
    mw.switch_auto_resume.set_active(cfg.auto_resume_on_reconnect);
    mw.switch_disconnect_peripherals.set_active(cfg.disconnect_peripherals);
    mw.switch_auto_start.set_active(cfg.auto_start_on_boot);
}

/// 从 UI 控件读取设置值（返回临时 Config 值，需要外部应用）
pub fn get_settings_values(mw: &MonitorWidgets) -> SettingsValues {
    SettingsValues {
        lock_delay_sec: mw.slider_lock_delay.value() as i32,
        rssi_threshold: mw.slider_rssi_threshold.value() as i32,
        auto_connect_device: mw.switch_auto_connect.is_active(),
        auto_lock_enabled: mw.switch_auto_lock.is_active(),
    }
}

pub struct SettingsValues {
    pub lock_delay_sec: i32,
    pub rssi_threshold: i32,
    pub auto_connect_device: bool,
    pub auto_lock_enabled: bool,
}

/// 显示运行日志弹窗（含 RSSI 信号曲线）
pub fn show_log_dialog(log_text: &str, rssi_curve: &str) {
    let dialog = gtk::Window::builder()
        .title(&tr("test_title"))
        .default_width(750)
        .default_height(600)
        .modal(true)
        .build();

    let vbox = GtkBox::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(10);
    vbox.set_margin_bottom(10);
    vbox.set_margin_start(10);
    vbox.set_margin_end(10);

    // ── RSSI 信号曲线 ──
    let curve_label = gtk::Label::new(Some(&tr("test_rssi_curve")));
    curve_label.add_css_class("title-label");
    curve_label.set_halign(gtk::Align::Start);
    vbox.append(&curve_label);

    let curve_scroll = gtk::ScrolledWindow::new();
    curve_scroll.set_min_content_height(200);
    let curve_view = gtk::TextView::new();
    curve_view.set_editable(false);
    curve_view.set_monospace(true);
    curve_view.buffer().set_text(rssi_curve);
    curve_view.add_css_class("log-textview");
    curve_scroll.set_child(Some(&curve_view));
    vbox.append(&curve_scroll);

    // ── 分隔 ──
    let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
    vbox.append(&sep);

    // ── 运行日志 ──
    let log_label = gtk::Label::new(Some(&tr("test_log")));
    log_label.add_css_class("title-label");
    log_label.set_halign(gtk::Align::Start);
    vbox.append(&log_label);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    let text_view = gtk::TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.buffer().set_text(log_text);
    text_view.add_css_class("log-textview");
    scroll.set_child(Some(&text_view));
    vbox.append(&scroll);

    // 关闭按钮
    let btn_close = Button::with_label("关闭");
    btn_close.set_halign(gtk::Align::End);
    let dialog_ref = dialog.clone();
    btn_close.connect_clicked(move |_| {
        dialog_ref.close();
    });
    vbox.append(&btn_close);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

// ── UI 刷新工具函数 ────────────────────────────────────────────────────────

pub fn set_status_dot_color(dot: &gtk::Label, status: &str) {
    let color = match status {
        "connected" => "#2ecc71",
        "fading" => "#f39c12",
        "disconnected" => "#95a5a6",
        "ble_off" => "#9b59b6",
        "reconnecting" => "#3498db",
        _ => "#95a5a6",
    };
    dot.set_markup(&format!("<span foreground=\"{color}\">●</span>"));
}

pub fn update_conn_status(labels: &gtk::Label, status: &str) {
    let text = match status {
        "connected" => tr("conn_connected"),
        "fading" => tr("conn_fading"),
        "disconnected" => tr("conn_disconnected"),
        "ble_off" => tr("conn_ble_off"),
        "reconnecting" => tr("conn_reconnecting"),
        _ => status.to_string(),
    };
    labels.set_text(&text);
}

pub fn update_signal_bar(bar: &gtk::ProgressBar, rssi: i32) {
    if rssi > -90 {
        let val = (rssi as f64 + 100.0) / 70.0;
        bar.set_fraction(val.clamp(0.0, 1.0));
    } else {
        bar.set_fraction(0.0);
    }
}

pub fn update_toggle_button(btn: &Button, state: &str) {
    let (label, class) = match state {
        "idle" => (tr("btn_start"), "btn-start"),
        "auto_paused" => (tr("btn_resume"), "btn-resume"),
        _ => (tr("btn_stop"), "btn-stop"),
    };
    btn.set_label(&label);
    btn.remove_css_class("btn-start");
    btn.remove_css_class("btn-stop");
    btn.remove_css_class("btn-resume");
    btn.add_css_class(class);
}
