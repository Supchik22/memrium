//! Checking a password against the system (PAM), for the lock screen. libpam is loaded when needed, so the
//! program neither links against it nor needs its headers; a machine without it just cannot unlock by password.

use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CString};

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_AUTH_ERR: c_int = 7;
const PAM_MAXTRIES: c_int = 8;

/// PAM asks questions; every password prompt gets our password, everything else no answer.
extern "C" fn conversation(count: c_int, messages: *mut *const PamMessage, responses: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    // SAFETY: PAM passes `count` valid message pointers, and `data` is the CString given to pam_start; the response
    // array and strings are allocated with malloc, which is how PAM frees them
    unsafe {
        let password = &*(data as *const CString);
        let out = libc::calloc(count.max(1) as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if out.is_null() {
            return PAM_BUF_ERR;
        }
        for i in 0..count as isize {
            let style = (**messages.offset(i)).msg_style;
            if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
                (*out.offset(i)).resp = libc::strdup(password.as_ptr());
            }
        }
        *responses = out;
        PAM_SUCCESS
    }
}

fn load() -> Result<Library, String> {
    let mut last = String::new();
    for name in ["libpam.so.0", "/run/current-system/sw/lib/libpam.so.0", "/usr/lib/libpam.so.0", "/lib/x86_64-linux-gnu/libpam.so.0", "/usr/lib64/libpam.so.0"] {
        // SAFETY: libpam has no load-time side effects beyond its own initialisation
        match unsafe { Library::new(name) } {
            Ok(l) => return Ok(l),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("libpam is not available ({last})"))
}

/// Does `password` belong to `user`? `Ok(false)` for a wrong password; `Err` when PAM itself cannot be used.
/// Blocks (a failed attempt is delayed by PAM on purpose): call it off the main thread.
pub fn authenticate(user: &str, password: &str) -> Result<bool, String> {
    let lib = load()?;
    let user = CString::new(user).map_err(|_| "bad user name")?;
    let password = CString::new(password).map_err(|_| "the password contains a NUL byte")?;
    // SAFETY: the signatures are those of libpam; `handle` is only used between pam_start and pam_end
    unsafe {
        let start: libloading::Symbol<unsafe extern "C" fn(*const c_char, *const c_char, *const PamConv, *mut *mut c_void) -> c_int> = lib.get(b"pam_start\0").map_err(|e| e.to_string())?;
        let auth: libloading::Symbol<unsafe extern "C" fn(*mut c_void, c_int) -> c_int> = lib.get(b"pam_authenticate\0").map_err(|e| e.to_string())?;
        let end: libloading::Symbol<unsafe extern "C" fn(*mut c_void, c_int) -> c_int> = lib.get(b"pam_end\0").map_err(|e| e.to_string())?;
        let conv = PamConv { conv: conversation, appdata_ptr: &password as *const CString as *mut c_void };
        let mut handle: *mut c_void = std::ptr::null_mut();
        // "login" exists on every distribution that has PAM; unlike a service of our own it needs no setup
        let service = CString::new("login").unwrap();
        let rc = start(service.as_ptr(), user.as_ptr(), &conv, &mut handle);
        if rc != PAM_SUCCESS {
            return Err(format!("pam_start failed ({rc})"));
        }
        let rc = auth(handle, 0);
        end(handle, rc);
        match rc {
            PAM_SUCCESS => Ok(true),
            r if r == PAM_AUTH_ERR || r == PAM_MAXTRIES || r == 10 /* user unknown */ => Ok(false),
            r => Err(format!("pam_authenticate returned {r}")),
        }
    }
}

#[cfg(test)]
mod tests {
    /// Needs a real PAM setup, so it only runs on request: `cargo test --lib -- --ignored pam`
    #[test]
    #[ignore]
    fn a_wrong_password_is_refused() {
        let user = std::env::var("USER").unwrap();
        assert_eq!(super::authenticate(&user, "definitely-not-the-password"), Ok(false));
    }
}
