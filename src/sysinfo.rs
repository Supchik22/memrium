//! Small bits of system state the shell displays: time, date, battery.

fn local_tm() -> libc::tm {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

/// "4:46 PM"
pub fn clock_time() -> String {
    let tm = local_tm();
    let h12 = if tm.tm_hour % 12 == 0 { 12 } else { tm.tm_hour % 12 };
    format!("{}:{:02} {}", h12, tm.tm_min, if tm.tm_hour < 12 { "AM" } else { "PM" })
}

/// "10/15/2026"
pub fn clock_date() -> String {
    let tm = local_tm();
    format!("{}/{}/{}", tm.tm_mon + 1, tm.tm_mday, tm.tm_year + 1900)
}

/// "Thu 15 Oct 4:46 pm" (menu bar style)
pub fn clock_menubar() -> String {
    let tm = local_tm();
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let h12 = if tm.tm_hour % 12 == 0 { 12 } else { tm.tm_hour % 12 };
    format!("{} {} {} {}:{:02} {}", days[tm.tm_wday as usize % 7], tm.tm_mday, months[tm.tm_mon as usize % 12], h12, tm.tm_min, if tm.tm_hour < 12 { "am" } else { "pm" })
}

/// Battery charge in percent and whether it is charging, if the machine has a battery.
pub fn battery() -> Option<(u8, bool)> {
    let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).ok()?.trim() == "Battery" {
            let cap: u8 = std::fs::read_to_string(p.join("capacity")).ok()?.trim().parse().ok()?;
            let charging = std::fs::read_to_string(p.join("status")).map(|s| s.trim() == "Charging").unwrap_or(false);
            return Some((cap, charging));
        }
    }
    None
}

/// Login name for the start menu.
pub fn user_name() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".into())
}
