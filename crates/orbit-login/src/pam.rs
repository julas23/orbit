//! PAM through libpam's C interface, declared here as in orbit-greet's
//! `pam.rs` (libpam.so.0 linked by name: building needs no -dev package),
//! with the session half a login service needs: credentials, opening and
//! closing the session, and the environment the modules set.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_TTY: c_int = 3;
const PAM_ESTABLISH_CRED: c_int = 0x0002;
const PAM_DELETE_CRED: c_int = 0x0004;
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
    fn pam_setcred(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_open_session(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_close_session(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut PamHandle, status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut PamHandle, errnum: c_int) -> *const c_char;
    fn pam_putenv(pamh: *mut PamHandle, name_value: *const c_char) -> c_int;
    fn pam_set_item(pamh: *mut PamHandle, item_type: c_int, item: *const c_void) -> c_int;
    fn pam_getenvlist(pamh: *mut PamHandle) -> *mut *mut c_char;
}

/// What the conversation answers with, and what PAM said along the way.
struct Conversation {
    /// The password, for a check; a session opens without one.
    password: Option<CString>,
    said: Vec<String>,
}

/// Answers PAM's prompts: the password to a hidden one, nothing to the
/// rest. Responses are allocated with malloc: PAM frees them.
extern "C" fn converse(n: c_int, msg: *mut *const PamMessage, resp: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    if n <= 0 || msg.is_null() || resp.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // Safety: PAM hands back the pointer given to pam_start, and `n`
    // messages in Linux-PAM's layout (an array of pointers).
    unsafe {
        let conv = &mut *(data as *mut Conversation);
        let replies = libc::calloc(n as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if replies.is_null() {
            return PAM_BUF_ERR;
        }
        for i in 0..n as usize {
            let m = &**msg.add(i);
            let text = if m.msg.is_null() { String::new() } else { CStr::from_ptr(m.msg).to_string_lossy().into_owned() };
            match m.msg_style {
                PAM_PROMPT_ECHO_OFF => match &conv.password {
                    Some(password) => (*replies.add(i)).resp = libc::strdup(password.as_ptr()),
                    None => {
                        log::warn!("PAM asked for a password where none is given: {text}");
                        libc::free(replies as *mut c_void);
                        return PAM_CONV_ERR;
                    }
                },
                PAM_PROMPT_ECHO_ON => {
                    log::warn!("PAM asked a question no one is there to answer: {text}");
                    (*replies.add(i)).resp = libc::strdup(c"".as_ptr());
                }
                PAM_ERROR_MSG | PAM_TEXT_INFO => conv.said.push(text),
                _ => {}
            }
        }
        *resp = replies;
    }
    PAM_SUCCESS
}

/// A PAM transaction, ended (and its session closed, if opened) on drop.
pub struct Pam {
    handle: *mut PamHandle,
    conv: Box<Conversation>,
    /// Kept alive for as long as the handle: PAM keeps the pointer.
    _pam_conv: Box<PamConv>,
    last: c_int,
    opened: bool,
}

impl Pam {
    pub fn start(service: &CStr, user: &str, password: Option<String>) -> Result<Self, String> {
        let user = CString::new(user).map_err(|_| "bad user name".to_owned())?;
        let password = password.map(CString::new).transpose().map_err(|_| "bad password".to_owned())?;
        let mut conv = Box::new(Conversation { password, said: Vec::new() });
        let pam_conv = Box::new(PamConv { conv: converse, appdata_ptr: &mut *conv as *mut Conversation as *mut c_void });
        let mut handle: *mut PamHandle = std::ptr::null_mut();
        // Safety: the strings and the conversation outlive the call; PAM
        // copies the user and keeps the conversation, kept in `Self`.
        let rc = unsafe { pam_start(service.as_ptr(), user.as_ptr(), &*pam_conv, &mut handle) };
        if rc != PAM_SUCCESS || handle.is_null() {
            return Err(format!("pam_start: {rc}"));
        }
        Ok(Self { handle, conv, _pam_conv: pam_conv, last: PAM_SUCCESS, opened: false })
    }

    fn check(&mut self, rc: c_int) -> Result<(), String> {
        self.last = rc;
        if rc == PAM_SUCCESS {
            return Ok(());
        }
        let said: Vec<String> = self.conv.said.drain(..).filter(|s| !s.trim().is_empty()).collect();
        if !said.is_empty() {
            return Err(said.join("\n"));
        }
        // Safety: a live handle; PAM's own static string.
        let text = unsafe { pam_strerror(self.handle, rc) };
        Err(if text.is_null() { format!("PAM error {rc}") } else { unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned() })
    }

    pub fn putenv(&mut self, name: &str, value: &str) {
        if let Ok(pair) = CString::new(format!("{name}={value}")) {
            // Safety: PAM copies the string.
            unsafe { pam_putenv(self.handle, pair.as_ptr()) };
        }
    }

    pub fn set_tty(&mut self, tty: &str) {
        if let Ok(tty) = CString::new(tty) {
            // Safety: PAM copies the item.
            unsafe { pam_set_item(self.handle, PAM_TTY, tty.as_ptr() as *const c_void) };
        }
    }

    /// The password, then the account (not expired, not locked).
    pub fn authenticate(&mut self) -> Result<(), String> {
        // Safety: a live handle.
        let rc = unsafe { pam_authenticate(self.handle, 0) };
        self.check(rc)?;
        self.account()
    }

    pub fn account(&mut self) -> Result<(), String> {
        // Safety: a live handle.
        let rc = unsafe { pam_acct_mgmt(self.handle, 0) };
        self.check(rc)
    }

    /// The credentials, then the session (pam_systemd registers it with
    /// logind, with this process as its leader).
    pub fn open_session(&mut self) -> Result<(), String> {
        // Safety: a live handle.
        let rc = unsafe { pam_setcred(self.handle, PAM_ESTABLISH_CRED) };
        self.check(rc)?;
        let rc = unsafe { pam_open_session(self.handle, 0) };
        if rc != PAM_SUCCESS {
            unsafe { pam_setcred(self.handle, PAM_DELETE_CRED) };
        }
        self.check(rc)?;
        self.opened = true;
        Ok(())
    }

    /// The environment the modules set (`XDG_SESSION_ID`,
    /// `XDG_RUNTIME_DIR`, the locale…).
    pub fn env(&self) -> Vec<(String, String)> {
        let mut env = Vec::new();
        // Safety: a malloc'd array of malloc'd strings, ending in null,
        // each freed here as PAM asks.
        unsafe {
            let list = pam_getenvlist(self.handle);
            if list.is_null() {
                return env;
            }
            let mut i = 0;
            loop {
                let entry = *list.add(i);
                if entry.is_null() {
                    break;
                }
                let text = CStr::from_ptr(entry).to_string_lossy().into_owned();
                if let Some((name, value)) = text.split_once('=') {
                    env.push((name.to_owned(), value.to_owned()));
                }
                libc::free(entry as *mut c_void);
                i += 1;
            }
            libc::free(list as *mut c_void);
        }
        env
    }
}

impl Drop for Pam {
    fn drop(&mut self) {
        // Safety: a live handle, ended once.
        unsafe {
            if self.opened {
                pam_close_session(self.handle, 0);
                pam_setcred(self.handle, PAM_DELETE_CRED);
            }
            pam_end(self.handle, self.last);
        }
        if let Some(password) = self.conv.password.take() {
            let mut bytes = password.into_bytes_with_nul();
            for b in bytes.iter_mut() {
                // Safety: a valid, exclusive pointer into the vector.
                unsafe { std::ptr::write_volatile(b, 0) };
            }
        }
    }
}
