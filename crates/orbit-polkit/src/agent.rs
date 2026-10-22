//! The agent on the system bus: it registers with polkitd for this
//! session (again whenever polkitd comes back) and serves
//! `org.freedesktop.PolicyKit1.AuthenticationAgent`. Each
//! `BeginAuthentication` is answered only when its dialog ends; they wait
//! their turn, one dialog at a time. Only polkitd is listened to.

use std::{
    any::Any,
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{dialog::Dialog, identity};

const AGENT_PATH: &str = "/io/github/julas23/Orbit/PolicyKit1/AuthenticationAgent";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";
pub const CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
const FAILED: &str = "org.freedesktop.PolicyKit1.Error.Failed";

const XML: &str = r#"<node>
  <interface name="org.freedesktop.PolicyKit1.AuthenticationAgent">
    <method name="BeginAuthentication">
      <arg type="s" name="action_id" direction="in"/>
      <arg type="s" name="message" direction="in"/>
      <arg type="s" name="icon_name" direction="in"/>
      <arg type="a{ss}" name="details" direction="in"/>
      <arg type="s" name="cookie" direction="in"/>
      <arg type="a(sa{sv})" name="identities" direction="in"/>
    </method>
    <method name="CancelAuthentication">
      <arg type="s" name="cookie" direction="in"/>
    </method>
  </interface>
</node>"#;

type Identities = Vec<(String, HashMap<String, glib::Variant>)>;

/// What polkitd asks: an action to allow, said in `message`.
pub struct Request {
    pub action: String,
    pub message: String,
    pub icon: String,
    pub details: Vec<(String, String)>,
    pub cookie: String,
    pub users: Vec<identity::User>,
    pub invocation: gio::DBusMethodInvocation,
}

pub struct Agent {
    app: gtk::Application,
    conn: gio::DBusConnection,
    /// polkitd's unique name, the only caller answered.
    authority: RefCell<Option<String>>,
    waiting: RefCell<VecDeque<Request>>,
    shown: RefCell<Option<Rc<Dialog>>>,
    keep: RefCell<Vec<Box<dyn Any>>>,
}

impl Agent {
    pub fn start(app: &gtk::Application) -> anyhow::Result<Rc<Self>> {
        let conn = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)?;
        let agent = Rc::new(Self {
            app: app.clone(),
            conn,
            authority: RefCell::new(None),
            waiting: RefCell::new(VecDeque::new()),
            shown: RefCell::new(None),
            keep: RefCell::new(Vec::new()),
        });

        let node = gio::DBusNodeInfo::for_xml(XML)?;
        let iface = node
            .lookup_interface("org.freedesktop.PolicyKit1.AuthenticationAgent")
            .ok_or_else(|| anyhow::anyhow!("no interface in the description"))?;
        let weak = Rc::downgrade(&agent);
        let id = agent
            .conn
            .register_object(AGENT_PATH, &iface)
            .method_call(move |_, sender, _, _, method, params, invocation| {
                let Some(agent) = weak.upgrade() else { return };
                if sender.is_none() || sender != agent.authority.borrow().as_deref() {
                    log::warn!("{method} from {sender:?}, not polkitd: refused");
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "only polkitd may ask");
                    return;
                }
                match method {
                    "BeginAuthentication" => agent.begin(&params, invocation),
                    "CancelAuthentication" => {
                        agent.cancel(&params.child_value(0).get::<String>().unwrap_or_default());
                        invocation.return_value(None);
                    }
                    _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
                }
            })
            .build()?;
        agent.keep.borrow_mut().push(Box::new(id));

        // Registered at once if polkitd is up, and again each time it
        // comes back (a restart forgets the agents).
        let (up, down) = (Rc::downgrade(&agent), Rc::downgrade(&agent));
        let watch = gio::bus_watch_name_on_connection(
            &agent.conn,
            AUTHORITY,
            gio::BusNameWatcherFlags::NONE,
            move |_, _, owner| {
                if let Some(agent) = up.upgrade() {
                    *agent.authority.borrow_mut() = Some(owner.to_owned());
                    agent.register();
                }
            },
            move |_, _| {
                log::warn!("polkitd left; waiting for it");
                if let Some(agent) = down.upgrade() {
                    *agent.authority.borrow_mut() = None;
                }
            },
        );
        agent.keep.borrow_mut().push(Box::new(watch));
        Ok(agent)
    }

    fn register(self: &Rc<Self>) {
        let subject = match subject(&self.conn) {
            Ok(s) => s,
            Err(err) => {
                log::error!("which session is this? {err}");
                return;
            }
        };
        log::info!("registering for session {}", subject.1.get("session-id").and_then(|v| v.get::<String>()).unwrap_or_default());
        let params = (subject, locale(), AGENT_PATH).to_variant();
        let app = self.app.clone();
        self.conn.call(
            Some(AUTHORITY),
            AUTHORITY_PATH,
            AUTHORITY_IFACE,
            "RegisterAuthenticationAgent",
            Some(&params),
            None,
            gio::DBusCallFlags::NONE,
            -1,
            gio::Cancellable::NONE,
            move |result| match result {
                Ok(_) => log::info!("registered with polkitd"),
                Err(err) => {
                    // Another agent (lxpolkit…) has the session: nothing
                    // to do here.
                    log::error!("polkitd refused the agent: {err}");
                    app.quit();
                }
            },
        );
    }

    fn begin(self: &Rc<Self>, params: &glib::Variant, invocation: gio::DBusMethodInvocation) {
        type Args = (String, String, String, HashMap<String, String>, String, Identities);
        let Some((action, message, icon, details, cookie, identities)) = params.get::<Args>() else {
            invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", "bad arguments");
            return;
        };
        let users = identity::users(&identities);
        if users.is_empty() {
            log::warn!("{action}: no user among {identities:?}");
            invocation.return_dbus_error(FAILED, "no identity to authenticate as");
            return;
        }
        log::info!("{action}: asked, as {}", users.iter().map(|u| u.name.as_str()).collect::<Vec<_>>().join(", "));
        let mut details: Vec<(String, String)> = details.into_iter().collect();
        details.sort();
        self.waiting.borrow_mut().push_back(Request { action, message, icon, details, cookie, users, invocation });
        self.next();
    }

    fn cancel(self: &Rc<Self>, cookie: &str) {
        let waiting = {
            let mut waiting = self.waiting.borrow_mut();
            waiting.iter().position(|r| r.cookie == cookie).and_then(|i| waiting.remove(i))
        };
        if let Some(request) = waiting {
            log::info!("{}: cancelled by polkitd before it was shown", request.action);
            request.invocation.return_dbus_error(CANCELLED, "cancelled by polkitd");
            return;
        }
        let shown = self.shown.borrow().clone();
        if let Some(dialog) = shown.filter(|d| d.cookie() == cookie) {
            log::info!("cancelled by polkitd");
            dialog.cancel();
        }
    }

    /// Shows the next request, if none is shown.
    fn next(self: &Rc<Self>) {
        if self.shown.borrow().is_some() {
            return;
        }
        let Some(request) = self.waiting.borrow_mut().pop_front() else { return };
        let weak = Rc::downgrade(self);
        let dialog = Dialog::show(&self.app, request, move || {
            if let Some(agent) = weak.upgrade() {
                agent.shown.borrow_mut().take();
                // After the dialog's own handlers are done.
                glib::idle_add_local_once(move || agent.next());
            }
        });
        *self.shown.borrow_mut() = Some(dialog);
    }
}

/// This login session, as polkitd names subjects: `XDG_SESSION_ID`, or
/// logind's answer for this process.
fn subject(conn: &gio::DBusConnection) -> anyhow::Result<(String, HashMap<String, glib::Variant>)> {
    let id = match std::env::var("XDG_SESSION_ID") {
        Ok(id) if !id.is_empty() => id,
        _ => {
            let login = |path: &str, iface: &str, method: &str, args: glib::Variant| {
                conn.call_sync(Some("org.freedesktop.login1"), path, iface, method, Some(&args), None, gio::DBusCallFlags::NONE, -1, gio::Cancellable::NONE)
            };
            let reply = login("/org/freedesktop/login1", "org.freedesktop.login1.Manager", "GetSessionByPID", (std::process::id(),).to_variant())?;
            let path = reply.child_value(0).str().unwrap_or_default().to_owned();
            let reply = login(&path, "org.freedesktop.DBus.Properties", "Get", ("org.freedesktop.login1.Session", "Id").to_variant())?;
            reply.child_value(0).as_variant().and_then(|v| v.get::<String>()).ok_or_else(|| anyhow::anyhow!("logind gave no session id"))?
        }
    };
    Ok(("unix-session".into(), HashMap::from([("session-id".to_owned(), id.to_variant())])))
}

/// The language polkitd writes its messages in.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "C".into())
}
