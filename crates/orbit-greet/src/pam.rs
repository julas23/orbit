//! Checking a password with PAM, through libpam's C interface (declared
//! here: libpam.so.0 is linked by name, so building needs no -dev
//! package). The service is `orbit-greet` (`/etc/pam.d/orbit-greet`;
//! without it PAM uses `other`). The greet runs as the session's user, so
//! pam_unix can check only that user's password (through unix_chkpwd), as
//! swaylock does. The right password also unlocks the login keyring
//! (`pam_gnome_keyring` in the service, PLANO-CORE §7.3 step 7), which
//! looks for the daemon through `XDG_RUNTIME_DIR` in the PAM environment.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

const SERVICE: &CStr = c"orbit-greet";

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;

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

type ConvFn = extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int;

#[repr(C)]
struct PamConv {
    conv: ConvFn,
    appdata_ptr: *mut c_void,
}

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[link(name = "libpam.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const PamConv, pamh: *mut *mut PamHandle) -> c_int;
    fn pam_authenticate(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut PamHandle, status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut PamHandle, errnum: c_int) -> *const c_char;
    fn pam_putenv(pamh: *mut PamHandle, name_value: *const c_char) -> c_int;
}

unsafe extern "C" {
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn strdup(s: *const c_char) -> *mut c_char;
}

/// What the conversation answers with, and what PAM said along the way.
struct Conversation {
    password: CString,
    said: Vec<String>,
}

/// Answers PAM's prompts: the password to a hidden one; nothing else is
/// asked of the greet. Responses are allocated with malloc: PAM frees them.
extern "C" fn converse(n: c_int, msg: *mut *const PamMessage, resp: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    if n <= 0 || msg.is_null() || resp.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // Safety: PAM hands back the pointer given to pam_start, and `n`
    // messages in Linux-PAM's layout (an array of pointers).
    unsafe {
        let conv = &mut *(data as *mut Conversation);
        let replies = calloc(n as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if replies.is_null() {
            return PAM_BUF_ERR;
        }
        for i in 0..n as usize {
            let m = &**msg.add(i);
            let text = if m.msg.is_null() { String::new() } else { CStr::from_ptr(m.msg).to_string_lossy().into_owned() };
            match m.msg_style {
                PAM_PROMPT_ECHO_OFF => (*replies.add(i)).resp = strdup(conv.password.as_ptr()),
                PAM_PROMPT_ECHO_ON => {
                    log::warn!("PAM asked a question the greet cannot answer: {text}");
                    (*replies.add(i)).resp = strdup(c"".as_ptr());
                }
                PAM_ERROR_MSG | PAM_TEXT_INFO => conv.said.push(text),
                _ => {}
            }
        }
        *resp = replies;
    }
    PAM_SUCCESS
}

/// Why the password was not accepted: PAM's own words when it said any,
/// else its error.
#[derive(Debug)]
pub struct Refused(pub String);

/// Checks `password` for `user` (authentication, then the account: not
/// expired, not locked). Blocks: pam_unix waits a moment after a wrong
/// password.
pub fn check(user: &str, password: String) -> Result<(), Refused> {
    let user = CString::new(user).map_err(|_| Refused("bad user name".into()))?;
    let password = CString::new(password).map_err(|_| Refused("bad password".into()))?;
    let mut conv = Box::new(Conversation { password, said: Vec::new() });
    let pam_conv = PamConv { conv: converse, appdata_ptr: &mut *conv as *mut Conversation as *mut c_void };
    let mut handle: *mut PamHandle = std::ptr::null_mut();
    // Safety: every pointer outlives the PAM transaction, which ends here.
    let result = unsafe {
        let rc = pam_start(SERVICE.as_ptr(), user.as_ptr(), &pam_conv, &mut handle);
        if rc != PAM_SUCCESS {
            return Err(Refused(format!("pam_start: {rc}")));
        }
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
            && let Ok(pair) = CString::new([b"XDG_RUNTIME_DIR=".as_slice(), dir.as_encoded_bytes()].concat())
        {
            pam_putenv(handle, pair.as_ptr());
        }
        let mut rc = pam_authenticate(handle, 0);
        if rc == PAM_SUCCESS {
            rc = pam_acct_mgmt(handle, 0);
        }
        let error = (rc != PAM_SUCCESS).then(|| {
            let text = pam_strerror(handle, rc);
            if text.is_null() { format!("PAM error {rc}") } else { CStr::from_ptr(text).to_string_lossy().into_owned() }
        });
        pam_end(handle, rc);
        error
    };
    let Conversation { password, said } = *conv;
    wipe(password);
    match result {
        None => Ok(()),
        Some(error) => {
            let said: Vec<String> = said.into_iter().filter(|s| !s.trim().is_empty()).collect();
            Err(Refused(if said.is_empty() { error } else { said.join("\n") }))
        }
    }
}

/// Overwrites the password before its memory is freed.
fn wipe(password: CString) {
    let mut bytes = password.into_bytes_with_nul();
    for b in bytes.iter_mut() {
        // Safety: a valid, exclusive pointer into the vector.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

/// The user the greet runs as: login name and the full name, if the
/// account has one (the GECOS field's first part).
pub fn user() -> (String, Option<String>) {
    let name = gtk::glib::user_name().to_string_lossy().into_owned();
    let real = gtk::glib::real_name().to_string_lossy().into_owned();
    let real = real.split(',').next().unwrap_or_default().trim().to_owned();
    let real = (!real.is_empty() && real != "Unknown" && real != name).then_some(real);
    (name, real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_password_is_refused() {
        let (user, _) = user();
        let refused = check(&user, "certainly not the password".into());
        assert!(refused.is_err(), "{refused:?}");
    }
}
