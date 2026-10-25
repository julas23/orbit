//! The keyring, over the Secret Service D-Bus API
//! (`org.freedesktop.secrets`): gnome-keyring today, oo7 or KeePassXC as
//! well, since apps only see the API. A "plain" session: the bus is the
//! user's own, as in orbit-pathfinder's import.
//!
//! A locked collection asks to be unlocked with a prompt the keyring shows
//! (orbit-prompter's card); the calls here wait for its answer.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, bail};
use gtk::{
    gio,
    glib::{self, Variant, prelude::*, variant::ObjectPath},
};
use zeroize::Zeroizing;

const BUS: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const SERVICE: &str = "org.freedesktop.Secret.Service";
const COLLECTION: &str = "org.freedesktop.Secret.Collection";
const ITEM: &str = "org.freedesktop.Secret.Item";
const PROMPT: &str = "org.freedesktop.Secret.Prompt";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// The schemas of what the safe itself stores.
pub const PASSWORD_SCHEMA: &str = "io.github.julas23.Orbit.Safe.Password";
pub const TOTP_SCHEMA: &str = "io.github.julas23.Orbit.Safe.Totp";

#[derive(Debug, Clone)]
pub struct Collection {
    pub path: String,
    pub label: String,
    pub locked: bool,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub path: String,
    /// The collection's label.
    pub collection: String,
    pub label: String,
    pub attributes: BTreeMap<String, String>,
}

impl Item {
    pub fn schema(&self) -> &str {
        self.attributes.get("xdg:schema").map_or("", String::as_str)
    }

    /// The safe's own (its fields can be edited; others' attributes are how
    /// their apps find them, and stay as they are).
    pub fn ours(&self) -> bool {
        matches!(self.schema(), PASSWORD_SCHEMA | TOTP_SCHEMA)
    }

    pub fn is_totp(&self) -> bool {
        self.schema() == TOTP_SCHEMA
    }

    /// The first attribute of `names` present.
    pub fn attribute(&self, names: &[&str]) -> Option<&str> {
        names.iter().find_map(|n| self.attributes.get(*n)).map(String::as_str).filter(|v| !v.is_empty())
    }

    pub fn username(&self) -> Option<&str> {
        self.attribute(&["username", "user", "account", "username_value", "login", "email"])
    }

    pub fn url(&self) -> Option<&str> {
        self.attribute(&["url", "origin_url", "server", "host", "domain", "signon_realm"])
    }
}

pub struct Service {
    conn: gio::DBusConnection,
    session: String,
}

impl Service {
    pub async fn connect() -> anyhow::Result<Self> {
        let conn = gio::bus_get_future(gio::BusType::Session).await.context("the session bus")?;
        let mut service = Service { conn, session: String::new() };
        let opened = service
            .call(ROOT, SERVICE, "OpenSession", ("plain", "".to_variant()).to_variant(), "(vo)")
            .await
            .context("no keyring answers on the bus (org.freedesktop.secrets)")?;
        service.session = opened.child_value(1).str().unwrap_or_default().to_owned();
        Ok(service)
    }

    async fn call(&self, path: &str, iface: &str, method: &str, args: Variant, reply: &str) -> anyhow::Result<Variant> {
        self.conn
            .call_future(
                Some(BUS),
                path,
                iface,
                method,
                Some(&args),
                Some(glib::VariantTy::new(reply).expect("valid reply type")),
                gio::DBusCallFlags::NONE,
                10_000,
            )
            .await
            .with_context(|| format!("{method} on {path}"))
    }

    async fn properties(&self, path: &str, iface: &str) -> anyhow::Result<HashMap<String, Variant>> {
        let all = self.call(path, PROPERTIES, "GetAll", (iface,).to_variant(), "(a{sv})").await?;
        Ok(all.child_value(0).get().unwrap_or_default())
    }

    async fn set(&self, path: &str, iface: &str, name: &str, value: Variant) -> anyhow::Result<()> {
        self.call(path, PROPERTIES, "Set", (iface, name, value).to_variant(), "()").await?;
        Ok(())
    }

    /// Every collection but the session's (in memory, gone at logout).
    pub async fn collections(&self) -> anyhow::Result<Vec<Collection>> {
        let props = self.properties(ROOT, SERVICE).await?;
        let paths: Vec<ObjectPath> = props.get("Collections").and_then(|v| v.get()).unwrap_or_default();
        let mut out = Vec::new();
        for path in paths {
            let path = path.as_str().to_owned();
            if path.ends_with("/session") {
                continue;
            }
            let props = self.properties(&path, COLLECTION).await?;
            out.push(Collection {
                label: props.get("Label").and_then(|v| v.get()).unwrap_or_default(),
                locked: props.get("Locked").and_then(|v| v.get()).unwrap_or(true),
                path,
            });
        }
        Ok(out)
    }

    /// The default collection ("login" on gnome-keyring), where new items go.
    pub async fn default_collection(&self) -> anyhow::Result<String> {
        let alias = self.call(ROOT, SERVICE, "ReadAlias", ("default",).to_variant(), "(o)").await?;
        let path = alias.child_value(0).str().unwrap_or("/").to_owned();
        if path == "/" {
            bail!("the keyring has no default collection");
        }
        Ok(path)
    }

    pub async fn items(&self, collection: &Collection) -> anyhow::Result<Vec<Item>> {
        let props = self.properties(&collection.path, COLLECTION).await?;
        let paths: Vec<ObjectPath> = props.get("Items").and_then(|v| v.get()).unwrap_or_default();
        let mut out = Vec::new();
        for path in paths {
            let props = match self.properties(path.as_str(), ITEM).await {
                Ok(props) => props,
                Err(err) => {
                    log::warn!("{err:#}");
                    continue;
                }
            };
            let attributes: HashMap<String, String> = props.get("Attributes").and_then(|v| v.get()).unwrap_or_default();
            out.push(Item {
                path: path.as_str().to_owned(),
                collection: collection.label.clone(),
                label: props.get("Label").and_then(|v| v.get()).unwrap_or_default(),
                attributes: attributes.into_iter().collect(),
            });
        }
        Ok(out)
    }

    /// Unlocks `paths` (collections or items); false when the prompt was
    /// dismissed.
    pub async fn unlock(&self, paths: &[String]) -> anyhow::Result<bool> {
        let paths: Vec<ObjectPath> = paths.iter().filter_map(|p| ObjectPath::try_from(p.clone()).ok()).collect();
        let reply = self.call(ROOT, SERVICE, "Unlock", (paths,).to_variant(), "(aoo)").await?;
        self.prompt(reply.child_value(1).str().unwrap_or("/")).await
    }

    /// Runs the keyring's prompt at `path` ("/" means none was needed) and
    /// waits for it: true when completed, false when dismissed.
    async fn prompt(&self, path: &str) -> anyhow::Result<bool> {
        if path == "/" {
            return Ok(true);
        }
        let (sender, completed) = async_channel::bounded(1);
        let _subscription = self.conn.subscribe_to_signal(
            Some(BUS),
            Some(PROMPT),
            Some("Completed"),
            Some(path),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let _ = sender.try_send(signal.parameters.get::<(bool, Variant)>());
            },
        );
        self.call(path, PROMPT, "Prompt", ("",).to_variant(), "()").await?;
        match completed.recv().await {
            Ok(Some((dismissed, _))) => Ok(!dismissed),
            Ok(None) => bail!("prompt: an unexpected answer"),
            Err(_) => bail!("prompt: the keyring went away"),
        }
    }

    pub async fn secret(&self, item: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let session = self.session_path()?;
        let reply = self.call(item, ITEM, "GetSecret", (session,).to_variant(), "((oayays))").await?;
        let (_, _, value, _): (ObjectPath, Vec<u8>, Vec<u8>, String) =
            reply.child_value(0).get().context("the secret's shape")?;
        Ok(Zeroizing::new(value))
    }

    pub async fn set_secret(&self, item: &str, value: &[u8]) -> anyhow::Result<()> {
        let secret = self.secret_value(value)?;
        self.call(item, ITEM, "SetSecret", Variant::tuple_from_iter([secret]), "()").await?;
        Ok(())
    }

    pub async fn set_label(&self, item: &str, label: &str) -> anyhow::Result<()> {
        self.set(item, ITEM, "Label", label.to_variant()).await
    }

    pub async fn set_attributes(&self, item: &str, attributes: &BTreeMap<String, String>) -> anyhow::Result<()> {
        let map: HashMap<String, String> = attributes.clone().into_iter().collect();
        self.set(item, ITEM, "Attributes", map.to_variant()).await
    }

    /// A new item in `collection`; returns its path.
    pub async fn create(
        &self,
        collection: &str,
        label: &str,
        attributes: &BTreeMap<String, String>,
        value: &[u8],
    ) -> anyhow::Result<String> {
        let map: HashMap<String, String> = attributes.clone().into_iter().collect();
        let props: HashMap<&str, Variant> = HashMap::from([
            ("org.freedesktop.Secret.Item.Label", label.to_variant()),
            ("org.freedesktop.Secret.Item.Attributes", map.to_variant()),
        ]);
        let secret = self.secret_value(value)?;
        let reply = self
            .call(collection, COLLECTION, "CreateItem", Variant::tuple_from_iter([props.to_variant(), secret, false.to_variant()]), "(oo)")
            .await?;
        let item = reply.child_value(0).str().unwrap_or("/").to_owned();
        let prompt = reply.child_value(1).str().unwrap_or("/").to_owned();
        if item == "/" && !self.prompt(&prompt).await? {
            bail!("the keyring did not create the item");
        }
        Ok(item)
    }

    pub async fn delete(&self, item: &str) -> anyhow::Result<()> {
        let reply = self.call(item, ITEM, "Delete", ().to_variant(), "(o)").await?;
        if !self.prompt(reply.child_value(0).str().unwrap_or("/")).await? {
            bail!("the keyring kept the item");
        }
        Ok(())
    }

    fn session_path(&self) -> anyhow::Result<ObjectPath> {
        ObjectPath::try_from(self.session.clone()).map_err(|_| anyhow::anyhow!("no keyring session"))
    }

    /// The Secret struct, `(oayays)`: a variant inside a tuple would go as
    /// `v`, so the calls that take it build their tuples by hand.
    fn secret_value(&self, value: &[u8]) -> anyhow::Result<Variant> {
        Ok((self.session_path()?, Vec::<u8>::new(), value.to_vec(), "text/plain; charset=utf8").to_variant())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the session's keyring: `cargo test -p orbit-safe -- --ignored`.
    /// Creates an item in the default collection, reads it back and deletes it.
    #[test]
    #[ignore]
    fn round_trip_in_the_keyring() {
        glib::MainContext::default().block_on(async {
            let service = Service::connect().await.unwrap();
            let collections = service.collections().await.unwrap();
            assert!(!collections.is_empty());
            let default = service.default_collection().await.unwrap();
            let attributes = BTreeMap::from([
                ("xdg:schema".to_owned(), TOTP_SCHEMA.to_owned()),
                ("issuer".to_owned(), "orbit-safe test".to_owned()),
            ]);
            let path = service.create(&default, "orbit-safe test", &attributes, b"GEZDGNBVGY3TQOJQ").await.unwrap();
            assert_eq!(service.secret(&path).await.unwrap().as_slice(), b"GEZDGNBVGY3TQOJQ");
            service.set_secret(&path, b"MFRGGZDF").await.unwrap();
            service.set_label(&path, "orbit-safe test 2").await.unwrap();
            let collection = collections.iter().find(|c| c.path == default).unwrap();
            let item = service.items(collection).await.unwrap().into_iter().find(|i| i.path == path).unwrap();
            assert_eq!(item.label, "orbit-safe test 2");
            assert!(item.is_totp() && item.ours());
            assert_eq!(service.secret(&path).await.unwrap().as_slice(), b"MFRGGZDF");
            service.delete(&path).await.unwrap();
            println!("collections: {:?}", collections.iter().map(|c| (&c.label, c.locked)).collect::<Vec<_>>());
        });
    }
}
