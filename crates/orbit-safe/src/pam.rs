//! Opening the safe: a PAM conversation (service `orbit-safe`,
//! `/etc/pam.d/orbit-safe`), held on a thread of its own and shown in the
//! window one message at a time. Everything the safe asks for to open comes
//! from that service: the password (common-auth), and whatever is added
//! there — a fingerprint (pam_fprintd), the face (howdy), a one-time code
//! from the phone (pam_google_authenticator) — each module's prompt shows
//! up as it asks. libpam.so.0 is linked by name, as in orbit-greet.
//!
//! The safe runs as its user, so pam_unix checks only that user's password
//! (through unix_chkpwd).

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    sync::mpsc,
};

const SERVICE: &CStr = c"orbit-safe";

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
}

unsafe extern "C" {
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn strdup(s: *const c_char) -> *mut c_char;
}

/// What the conversation tells the window.
#[derive(Debug)]
pub enum Event {
    /// A question: shown (`echo`, a code) or hidden (a password).
    Ask { text: String, echo: bool },
    /// A module's words: "Place your finger on the reader", an error.
    Say { text: String, error: bool },
    /// The end: open, or refused with PAM's reason.
    Done(Result<(), String>),
}

struct Conversation {
    events: async_channel::Sender<Event>,
    answers: mpsc::Receiver<String>,
}

/// Hands each message to the window and, for a question, waits for its
/// answer. Responses are allocated with malloc: PAM frees them.
extern "C" fn converse(n: c_int, msg: *mut *const PamMessage, resp: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    if n <= 0 || msg.is_null() || resp.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // Safety: PAM hands back the pointer given to pam_start, and `n`
    // messages in Linux-PAM's layout (an array of pointers).
    unsafe {
        let conv = &*(data as *const Conversation);
        let replies = calloc(n as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if replies.is_null() {
            return PAM_BUF_ERR;
        }
        for i in 0..n as usize {
            let m = &**msg.add(i);
            let text = if m.msg.is_null() { String::new() } else { CStr::from_ptr(m.msg).to_string_lossy().trim().to_owned() };
            match m.msg_style {
                PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => {
                    let echo = m.msg_style == PAM_PROMPT_ECHO_ON;
                    // The window gone: nothing will answer.
                    let answer = match conv.events.send_blocking(Event::Ask { text, echo }) {
                        Ok(()) => conv.answers.recv().ok(),
                        Err(_) => None,
                    };
                    let Some(answer) = answer else {
                        free_replies(replies, i);
                        return PAM_CONV_ERR;
                    };
                    let answer = CString::new(answer).unwrap_or_default();
                    (*replies.add(i)).resp = strdup(answer.as_ptr());
                    wipe(answer);
                }
                PAM_ERROR_MSG | PAM_TEXT_INFO if !text.is_empty() => {
                    let _ = conv.events.send_blocking(Event::Say { text, error: m.msg_style == PAM_ERROR_MSG });
                }
                _ => {}
            }
        }
        *resp = replies;
    }
    PAM_SUCCESS
}

/// Frees the answers given before a failed question, and the array.
unsafe fn free_replies(replies: *mut PamResponse, given: usize) {
    unsafe extern "C" {
        fn free(p: *mut c_void);
    }
    // Safety: the first `given` answers were made by strdup, the array by
    // calloc; none was handed to PAM.
    unsafe {
        for i in 0..given {
            free((*replies.add(i)).resp as *mut c_void);
        }
        free(replies as *mut c_void);
    }
}

/// Starts a PAM transaction for the user the safe runs as. The window gets
/// its messages on the returned receiver and answers on the sender; the
/// last message is `Done`.
pub fn start() -> (async_channel::Receiver<Event>, mpsc::Sender<String>) {
    let (events, received) = async_channel::unbounded();
    let (answer, answers) = mpsc::channel();
    let user = gtk::glib::user_name().to_string_lossy().into_owned();
    let thread = std::thread::Builder::new().name("pam".into()).spawn({
        let events = events.clone();
        move || {
            let done = authenticate(&user, Conversation { events: events.clone(), answers });
            let _ = events.send_blocking(Event::Done(done));
        }
    });
    if let Err(err) = thread {
        let _ = events.send_blocking(Event::Done(Err(err.to_string())));
    }
    (received, answer)
}

/// Authentication, then the account (not expired, not locked). Blocks:
/// pam_unix waits a moment after a wrong password.
fn authenticate(user: &str, conv: Conversation) -> Result<(), String> {
    let user = CString::new(user).map_err(|_| "bad user name".to_owned())?;
    let conv = Box::new(conv);
    let pam_conv = PamConv { conv: converse, appdata_ptr: &*conv as *const Conversation as *mut c_void };
    let mut handle: *mut PamHandle = std::ptr::null_mut();
    // Safety: every pointer outlives the PAM transaction, which ends here.
    unsafe {
        let rc = pam_start(SERVICE.as_ptr(), user.as_ptr(), &pam_conv, &mut handle);
        if rc != PAM_SUCCESS {
            return Err(format!("pam_start: {rc}"));
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
        error.map_or(Ok(()), Err)
    }
}

/// Overwrites an answer before its memory is freed.
fn wipe(answer: CString) {
    let mut bytes = answer.into_bytes_with_nul();
    for b in bytes.iter_mut() {
        // Safety: a valid, exclusive pointer into the vector.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}
