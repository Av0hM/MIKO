use serde_json::{Value, json};
use std::ffi::CString;
use std::os::raw::c_char;
use std::process::Command;
use std::sync::Mutex;

static INITIALIZED: Mutex<bool> = Mutex::new(false);

fn read_intel_gpu() -> Option<Value> {
    let output = Command::new("sudo")
        .args([
            "-n",
            "intel_gpu_top",
            "-J",
            "-s",
            "100",
            "-n",
            "2",
            "-o",
            "-",
        ])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v.is_object() {
                return Some(v);
            }
        }
    }
    None
}

fn read_sysfs_intel() -> Option<Value> {
    let freq_cur = std::fs::read_to_string("/sys/class/drm/card1/gt_cur_freq_mhz")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let freq_max = std::fs::read_to_string("/sys/class/drm/card1/gt_max_freq_mhz")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let freq_act = std::fs::read_to_string("/sys/class/drm/card1/gt_act_freq_mhz")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(json!({
        "vendor": "intel",
        "freq_cur_mhz": freq_cur,
        "freq_max_mhz": freq_max,
        "freq_act_mhz": freq_act,
    }))
}

fn read_nvidia() -> Option<Value> {
    let temp_out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    let temp: f32 = String::from_utf8_lossy(&temp_out.stdout)
        .trim()
        .parse()
        .ok()?;

    let mem_out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    let mem_str = String::from_utf8_lossy(&mem_out.stdout);
    let parts: Vec<&str> = mem_str.trim().split(',').collect();
    if parts.len() != 2 {
        return None;
    }
    let used: u64 = parts[0].trim().parse().ok()?;
    let total: u64 = parts[1].trim().parse().ok()?;

    Some(json!({
        "vendor": "nvidia",
        "temp_c": temp,
        "mem_used_mb": used,
        "mem_total_mb": total,
    }))
}

fn read_amd() -> Option<Value> {
    let output = Command::new("rocm-smi")
        .args(["--showtemp"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("Temperature") && line.contains("°C") {
            for part in line.split_whitespace() {
                if let Some(temp_str) = part.strip_suffix("°C") {
                    if let Ok(temp) = temp_str.parse::<f32>() {
                        return Some(json!({"vendor": "amd", "temp_c": temp}));
                    }
                }
            }
        }
    }
    None
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_name() -> *const c_char {
    c"gpu_monitor".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_version() -> *const c_char {
    c"0.1.0".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_description() -> *const c_char {
    c"Monitors GPU stats via sysfs and vendor utilities".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_init() -> i32 {
    *INITIALIZED.lock().unwrap() = true;
    println!("GPU Plugin initialized");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_update() -> *mut c_char {
    if !*INITIALIZED.lock().unwrap() {
        return std::ptr::null_mut();
    }

    let data = read_sysfs_intel()
        .or_else(read_intel_gpu)
        .or_else(read_nvidia)
        .or_else(read_amd)
        .unwrap_or_else(|| json!({}));

    let json_str = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());
    CString::new(json_str).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_shutdown() -> i32 {
    *INITIALIZED.lock().unwrap() = false;
    println!("GPU Plugin shutdown");
    0
}
