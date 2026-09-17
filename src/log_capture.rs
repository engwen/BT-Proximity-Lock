/// 日志捕获模块：捕获应用日志 + RSSI 历史供诊断查看
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;

const MAX_LOG_ENTRIES: usize = 500;
const MAX_RSSI_POINTS: usize = 120;

static LOG_BUFFER: Mutex<Option<VecDeque<String>>> = Mutex::new(None);
static RSSI_HISTORY: Mutex<VecDeque<i32>> = Mutex::new(VecDeque::new());

pub fn init() -> Result<(), SetLoggerError> {
    *LOG_BUFFER.lock().unwrap() = Some(VecDeque::with_capacity(256));
    log::set_boxed_logger(Box::new(CaptureLogger))
        .map(|()| log::set_max_level(LevelFilter::Debug))
}

/// 记录一个 RSSI 数据点
pub fn push_rssi(rssi: i32) {
    let mut buf = RSSI_HISTORY.lock().unwrap();
    if buf.len() >= MAX_RSSI_POINTS {
        buf.pop_front();
    }
    buf.push_back(rssi);
}

/// 获取 RSSI 历史数据的克隆
pub fn get_rssi_history() -> Vec<i32> {
    RSSI_HISTORY.lock().unwrap().iter().cloned().collect()
}

pub fn get_logs() -> String {
    let guard = LOG_BUFFER.lock().unwrap();
    match guard.as_ref() {
        Some(buf) => buf.iter().cloned().collect::<Vec<_>>().join("\n"),
        None => String::new(),
    }
}

/// 生成 RSSI 信号曲线的文本可视化
pub fn render_rssi_curve(history: &[i32], width: usize, height: usize) -> String {
    if history.is_empty() {
        return "暂无 RSSI 数据，启动监控后自动采集".to_string();
    }

    // 取最后 width 个点
    let data: Vec<i32> = if history.len() > width {
        history[history.len() - width..].to_vec()
    } else {
        history.to_vec()
    };

    let min_val = data.iter().copied().min().unwrap_or(-100).min(-100);
    let max_val = data.iter().copied().max().unwrap_or(-30).max(-30);
    let range = (max_val - min_val).max(1);

    // 构建画布
    let mut canvas = vec![vec![' '; width]; height];

    for (x, &val) in data.iter().enumerate() {
        let normalized = (val - min_val) as f64 / range as f64;
        let y = ((1.0 - normalized) * (height - 1) as f64).round() as usize;
        let y = y.min(height - 1);
        canvas[y][x] = '●';
        // 连线上下的点
        for row in 0..height {
            if canvas[row][x] == ' ' {
                if row > 0 && canvas[row - 1][x] == '●' {
                    canvas[row][x] = '│';
                } else if row + 1 < height && canvas[row + 1][x] == '●' {
                    canvas[row][x] = '│';
                }
            }
        }
    }

    let mut lines = Vec::new();
    // Y 轴标签（顶部、中间、底部）
    for (row_idx, row) in canvas.iter().enumerate() {
        let label = match row_idx {
            0 => format!("{:>4} |", max_val),
            r if r == height / 2 => format!("{:>4} |", (max_val + min_val) / 2),
            r if r == height - 1 => format!("{:>4} |", min_val),
            _ => "     |".to_string(),
        };
        let line: String = row.iter().collect();
        lines.push(format!("{label}{line}"));
    }
    // X 轴
    lines.push(format!("     +{}", "-".repeat(width)));
    lines.push(format!("      {}个数据点，最新: {} dBm", data.len(), data.last().unwrap_or(&0)));
    lines.join("\n")
}

struct CaptureLogger;

impl Log for CaptureLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ts = chrono_like_timestamp();
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => " WARN",
            Level::Info => " INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };
        let msg = format!("{ts} {level}  {}", record.args());

        // 写入缓冲区
        if let Some(buf) = LOG_BUFFER.lock().unwrap().as_mut() {
            if buf.len() >= MAX_LOG_ENTRIES {
                let drain = buf.len() - MAX_LOG_ENTRIES + 64;
                buf.drain(..drain);
            }
            buf.push_back(msg.clone());
        }

        // 同时输出到 stderr（使用 eprint! 避免递归触发 log 宏）
        let _ = std::io::stderr().write_all(msg.as_bytes());
        let _ = std::io::stderr().write_all(b"\n");
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

/// 简单的时间戳生成（避免引入 chrono 依赖）
fn chrono_like_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 简化为 UTC 时间戳
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    // 从 epoch 天数计算年月日（近似）
    let (y, mo, d) = epoch_days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn epoch_days_to_ymd(days: u64) -> (u64, u64, u64) {
    // 简化的日期计算
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
