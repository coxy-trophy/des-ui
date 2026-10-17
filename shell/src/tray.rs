//! The system tray: applications' status icons (StatusNotifierItem) in the status strip.
//!
//! The shell owns `org.kde.StatusNotifierWatcher` when nobody else does, and is always a
//! host. Every D-Bus call happens on one background thread; the strip reads a snapshot
//! behind a mutex and is woken when it changes. Menus come over com.canonical.dbusmenu.
//!
//! Specification: https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/

use crate::{app_icons::IconCache, theme::Theme, ui};
use eframe::egui::{self, ColorImage, Rect, Sense, Stroke, TextureHandle, Ui, vec2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::blocking::{Connection, MessageIterator};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
/// The size icons are drawn at, and the pixmap size preferred from those offered.
const ICON: f32 = 18.0;
const PIXMAP_TARGET: i32 = 22;
/// Pictures larger than this are refused rather than decoded.
const PIXMAP_MAX_SIDE: i32 = 512;
/// Applications that never answer must not hold the tray up.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_ITEMS: usize = 64;
const MAX_MENU_NODES: usize = 1000;

/// One entry of an application's menu.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    /// `Some(on)` for a check mark or a radio button.
    pub toggle: Option<bool>,
    pub children: Vec<MenuNode>,
}

/// What the strip draws for one application.
#[derive(Clone, Default)]
pub struct Item {
    key: String,
    bus: String,
    path: String,
    /// The bus's unique name, which its signals arrive from.
    owner: String,
    title: String,
    tooltip: String,
    status: String,
    /// A file path or an icon name, for the shared icon cache.
    icon: Option<String>,
    /// Width and height, and RGBA bytes.
    pixmap: Option<Arc<([usize; 2], Vec<u8>)>>,
    generation: u64,
    menu_path: Option<String>,
    item_is_menu: bool,
    menu: Option<Vec<MenuNode>>,
}

enum Command {
    Activate { key: String, x: i32, y: i32 },
    Secondary { key: String, x: i32, y: i32 },
    Scroll { key: String, delta: i32 },
    MenuOpened(String),
    MenuClicked { key: String, id: i32 },
}

enum Event {
    Refresh(String),
    Forget(String),
    /// A bus name lost its owner.
    Gone(String),
    Changed { owner: String, path: String },
    MenuChanged { owner: String, path: String },
    Command(Command),
}

pub struct Tray {
    shared: Arc<Mutex<Vec<Item>>>,
    events: Mutex<Sender<Event>>,
    icons: RefCell<IconCache>,
    pixmaps: RefCell<HashMap<String, (u64, TextureHandle)>>,
}

impl Tray {
    /// Starts watching and hosting on the session bus. Without a bus the strip stays empty.
    pub fn start(ctx: egui::Context) -> Tray {
        let shared: Arc<Mutex<Vec<Item>>> = Arc::default();
        let (events, inbox) = mpsc::channel();
        let (worker_shared, worker_events) = (shared.clone(), events.clone());
        std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || {
                if let Err(error) = run(worker_shared, worker_events, inbox, ctx) {
                    eprintln!("des-ui: no system tray: {error}");
                }
            })
            .ok();
        Tray { shared, events: Mutex::new(events), icons: RefCell::new(IconCache::new()), pixmaps: RefCell::default() }
    }

    fn send(&self, command: Command) {
        let _ = self.events.lock().unwrap().send(Event::Command(command));
    }
}

// ---------------------------------------------------------------------------------------
// Pure parts, tested below.

/// Where an item lives, from what it registered with: a bus name, an object path (the
/// sender's own), or `bus/path` as other watchers list them.
pub fn item_address(service: &str, sender: Option<&str>) -> Option<(String, String)> {
    let service = service.trim();
    if service.is_empty() {
        return None;
    }
    if service.starts_with('/') {
        return Some((sender?.to_string(), service.to_string()));
    }
    match service.split_once('/') {
        Some((bus, path)) if !bus.is_empty() => Some((bus.to_string(), format!("/{path}"))),
        Some(_) => None,
        None => Some((service.to_string(), "/StatusNotifierItem".to_string())),
    }
}

fn key_of(bus: &str, path: &str) -> String {
    format!("{bus}{path}")
}

fn split_key(key: &str) -> Option<(&str, &str)> {
    let at = key.find('/')?;
    Some((&key[..at], &key[at..]))
}

/// The watcher's list of registered items.
#[derive(Default, Debug)]
pub struct Registry {
    items: Vec<String>,
}

impl Registry {
    /// Adds an item; whether it is new.
    pub fn register(&mut self, key: &str) -> bool {
        if self.items.iter().any(|item| item == key) || self.items.len() >= MAX_ITEMS {
            return false;
        }
        self.items.push(key.to_string());
        true
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item != key);
        self.items.len() != before
    }

    /// Removes every item on a bus name that has gone; the ones removed.
    pub fn remove_owner(&mut self, bus: &str) -> Vec<String> {
        let prefix = format!("{bus}/");
        let (gone, kept): (Vec<String>, Vec<String>) = std::mem::take(&mut self.items).into_iter().partition(|item| item.starts_with(&prefix));
        self.items = kept;
        gone
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }
}

/// A Passive item says it has nothing to show.
pub fn visible(status: &str) -> bool {
    status != "Passive"
}

pub fn attention(status: &str) -> bool {
    status == "NeedsAttention"
}

/// A menu label without its access-key underscores; `__` is a literal underscore.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The picture nearest `target` pixels wide among those offered (ties go to the larger),
/// converted from ARGB32 in network byte order to RGBA.
pub fn pick_pixmap(offered: &[(i32, i32, Vec<u8>)], target: i32) -> Option<([usize; 2], Vec<u8>)> {
    let (width, height, bytes) = offered
        .iter()
        .filter(|(w, h, bytes)| {
            *w > 0 && *h > 0 && *w <= PIXMAP_MAX_SIDE && *h <= PIXMAP_MAX_SIDE && bytes.len() == (*w as usize) * (*h as usize) * 4
        })
        .min_by_key(|(w, _, _)| ((w - target).abs(), -w))?;
    let rgba = bytes.chunks_exact(4).flat_map(|argb| [argb[1], argb[2], argb[3], argb[0]]).collect();
    Some(([*width as usize, *height as usize], rgba))
}

fn unwrap<'v, 'a>(value: &'v Value<'a>) -> &'v Value<'a> {
    match value {
        Value::Value(inner) => unwrap(inner),
        other => other,
    }
}

fn text(value: &Value<'_>) -> Option<String> {
    match unwrap(value) {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(p) => Some(p.to_string()),
        _ => None,
    }
}

fn int(value: &Value<'_>) -> Option<i64> {
    match unwrap(value) {
        Value::I32(n) => Some(*n as i64),
        Value::U32(n) => Some(*n as i64),
        Value::I64(n) => Some(*n),
        Value::U8(n) => Some(*n as i64),
        _ => None,
    }
}

fn boolean(value: &Value<'_>) -> Option<bool> {
    match unwrap(value) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// `a(iiay)`: the pictures an item offers.
pub fn pixmaps(value: &Value<'_>) -> Vec<(i32, i32, Vec<u8>)> {
    let Value::Array(array) = unwrap(value) else { return Vec::new() };
    array
        .inner()
        .iter()
        .filter_map(|entry| {
            let Value::Structure(structure) = unwrap(entry) else { return None };
            let [w, h, data] = structure.fields() else { return None };
            let Value::Array(data) = unwrap(data) else { return None };
            let bytes = data.inner().iter().map(|byte| match byte { Value::U8(b) => Some(*b), _ => None }).collect::<Option<Vec<u8>>>()?;
            Some((int(w)? as i32, int(h)? as i32, bytes))
        })
        .collect()
}

/// `(sa(iiay)ss)`: a tooltip's title and text, joined for display.
pub fn tooltip(value: &Value<'_>) -> String {
    let Value::Structure(structure) = unwrap(value) else { return String::new() };
    let fields = structure.fields();
    let part = |n: usize| fields.get(n).and_then(text).map(|s| crate::notify::plain(&s)).unwrap_or_default();
    let (title, body) = (part(2), part(3));
    match (title.is_empty(), body.is_empty()) {
        (false, false) => format!("{title}\n{body}"),
        (false, true) => title,
        (true, false) => body,
        (true, true) => String::new(),
    }
}

/// A dbusmenu entry from its id, properties and children.
fn node(id: i32, properties: &[(String, &Value<'_>)], children: &[Value<'_>], budget: &mut usize) -> MenuNode {
    let property = |name: &str| properties.iter().find(|(key, _)| key == name).map(|(_, value)| *value);
    let toggle_type = property("toggle-type").and_then(text).unwrap_or_default();
    let toggle = (toggle_type == "checkmark" || toggle_type == "radio")
        .then(|| property("toggle-state").and_then(int).unwrap_or(0) == 1);
    let mut kids = Vec::new();
    for child in children {
        if *budget == 0 {
            break;
        }
        if let Some(child) = parse_node(child, budget) {
            kids.push(child);
        }
    }
    MenuNode {
        id,
        label: strip_mnemonic(&property("label").and_then(text).unwrap_or_default()),
        enabled: property("enabled").and_then(boolean).unwrap_or(true),
        visible: property("visible").and_then(boolean).unwrap_or(true),
        separator: property("type").and_then(text).as_deref() == Some("separator"),
        toggle,
        children: kids,
    }
}

/// One `(ia{sv}av)` entry of a GetLayout answer.
pub fn parse_node(value: &Value<'_>, budget: &mut usize) -> Option<MenuNode> {
    *budget = budget.checked_sub(1)?;
    let Value::Structure(structure) = unwrap(value) else { return None };
    let [id, properties, children] = structure.fields() else { return None };
    let id = int(id)? as i32;
    let Value::Dict(dict) = unwrap(properties) else { return None };
    let properties: Vec<(String, &Value<'_>)> = dict.iter().filter_map(|(key, value)| Some((text(key)?, value))).collect();
    let Value::Array(children) = unwrap(children) else { return None };
    Some(node(id, &properties, children.inner(), budget))
}

// ---------------------------------------------------------------------------------------
// The watcher, served when no one else is.

struct Watcher {
    registry: Arc<Mutex<Registry>>,
    events: Mutex<Sender<Event>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header.sender().map(|sender| sender.to_string());
        let (bus, path) = item_address(service, sender.as_deref()).ok_or_else(|| zbus::fdo::Error::InvalidArgs("no item address".into()))?;
        let key = key_of(&bus, &path);
        let new = self.registry.lock().unwrap().register(&key);
        if new {
            let _ = Self::status_notifier_item_registered(&emitter, &key).await;
        }
        let _ = self.events.lock().unwrap().send(Event::Refresh(key));
        Ok(())
    }

    async fn register_status_notifier_host(&self, _service: &str, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registry.lock().unwrap().items().to_vec()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------------------
// The background thread.

/// Forwards matching signals as events, until the bus or the tray goes away.
fn listen(connection: &Connection, rule: &'static str, events: Sender<Event>, map: fn(&zbus::Message) -> Option<Event>) {
    let connection = connection.clone();
    std::thread::Builder::new()
        .name("tray-signals".into())
        .spawn(move || {
            let Ok(iterator) = MessageIterator::for_match_rule(rule, &connection, Some(256)) else { return };
            for message in iterator.flatten() {
                if let Some(event) = map(&message) {
                    if events.send(event).is_err() {
                        return;
                    }
                }
            }
        })
        .ok();
}

fn signal_source(message: &zbus::Message) -> Option<(String, String)> {
    let header = message.header();
    Some((header.sender()?.to_string(), header.path()?.to_string()))
}

fn run(shared: Arc<Mutex<Vec<Item>>>, events: Sender<Event>, inbox: Receiver<Event>, ctx: egui::Context) -> zbus::Result<()> {
    let connection = zbus::blocking::connection::Builder::session()?.method_timeout(CALL_TIMEOUT).build()?;
    let registry: Arc<Mutex<Registry>> = Arc::default();
    connection
        .object_server()
        .at(WATCHER_PATH, Watcher { registry: registry.clone(), events: Mutex::new(events.clone()) })?;
    let owning = matches!(
        connection.request_name_with_flags(WATCHER, zbus::fdo::RequestNameFlags::DoNotQueue.into()),
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner)
    );
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    let _ = connection.request_name(host.as_str());

    listen(&connection, "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged'", events.clone(), |message| {
        let (name, _old, new): (String, String, String) = message.body().deserialize().ok()?;
        new.is_empty().then_some(Event::Gone(name))
    });
    listen(&connection, "type='signal',interface='org.kde.StatusNotifierItem'", events.clone(), |message| {
        let (owner, path) = signal_source(message)?;
        Some(Event::Changed { owner, path })
    });
    listen(&connection, "type='signal',interface='com.canonical.dbusmenu'", events.clone(), |message| {
        let member = message.header().member()?.to_string();
        if member != "LayoutUpdated" && member != "ItemsPropertiesUpdated" {
            return None;
        }
        let (owner, path) = signal_source(message)?;
        Some(Event::MenuChanged { owner, path })
    });

    if !owning {
        // Another watcher is in charge: list what it has, and follow it.
        listen(&connection, "type='signal',interface='org.kde.StatusNotifierWatcher'", events.clone(), |message| {
            let member = message.header().member()?.to_string();
            let (service,): (String,) = message.body().deserialize().ok()?;
            let (bus, path) = item_address(&service, None)?;
            match member.as_str() {
                "StatusNotifierItemRegistered" => Some(Event::Refresh(key_of(&bus, &path))),
                "StatusNotifierItemUnregistered" => Some(Event::Forget(key_of(&bus, &path))),
                _ => None,
            }
        });
        let _ = connection.call_method(Some(WATCHER), WATCHER_PATH, Some(WATCHER), "RegisterStatusNotifierHost", &(host.as_str(),));
        if let Ok(reply) = connection.call_method(Some(WATCHER), WATCHER_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(WATCHER, "RegisteredStatusNotifierItems")) {
            if let Ok(value) = reply.body().deserialize::<OwnedValue>() {
                if let Value::Array(array) = unwrap(&value) {
                    for service in array.inner().iter().filter_map(text) {
                        if let Some((bus, path)) = item_address(&service, None) {
                            let _ = events.send(Event::Refresh(key_of(&bus, &path)));
                        }
                    }
                }
            }
        }
    }

    let mut items: Vec<Item> = Vec::new();
    let mut generation = 0u64;
    for event in inbox {
        let changed = match event {
            Event::Refresh(key) => {
                generation += 1;
                refresh(&connection, &mut items, &key, generation, owning.then_some(&registry))
            }
            Event::Forget(key) => {
                let before = items.len();
                items.retain(|item| item.key != key);
                before != items.len()
            }
            Event::Gone(name) => {
                if owning {
                    let gone = registry.lock().unwrap().remove_owner(&name);
                    for key in gone {
                        let _ = connection.emit_signal(None::<&str>, WATCHER_PATH, WATCHER, "StatusNotifierItemUnregistered", &(key.as_str(),));
                    }
                }
                let before = items.len();
                items.retain(|item| item.bus != name && item.owner != name);
                before != items.len()
            }
            Event::Changed { owner, path } => {
                let keys: Vec<String> = items.iter().filter(|item| item.owner == owner && item.path == path).map(|item| item.key.clone()).collect();
                let mut any = false;
                for key in keys {
                    generation += 1;
                    any |= refresh(&connection, &mut items, &key, generation, owning.then_some(&registry));
                }
                any
            }
            Event::MenuChanged { owner, path } => {
                let mut any = false;
                for item in items.iter_mut().filter(|item| item.owner == owner && item.menu_path.as_deref() == Some(path.as_str())) {
                    item.menu = fetch_menu(&connection, &item.bus, &path);
                    any = true;
                }
                any
            }
            Event::Command(command) => {
                act(&connection, &mut items, command);
                true
            }
        };
        if changed {
            *shared.lock().unwrap() = items.clone();
            ctx.request_repaint();
        }
    }
    Ok(())
}

/// Reads an item afresh; whether anything was published. An item that no longer answers
/// is dropped.
fn refresh(connection: &Connection, items: &mut Vec<Item>, key: &str, generation: u64, registry: Option<&Arc<Mutex<Registry>>>) -> bool {
    let Some((bus, path)) = split_key(key) else { return false };
    let reply = connection.call_method(Some(bus), path, Some("org.freedesktop.DBus.Properties"), "GetAll", &(ITEM,));
    let Some(properties) = reply.ok().and_then(|reply| reply.body().deserialize::<HashMap<String, OwnedValue>>().ok()) else {
        let before = items.len();
        items.retain(|item| item.key != key);
        if let Some(registry) = registry {
            registry.lock().unwrap().remove(key);
        }
        return before != items.len();
    };
    if !items.iter().any(|item| item.key == key) && items.len() >= MAX_ITEMS {
        return false;
    }
    let owner = if bus.starts_with(':') {
        bus.to_string()
    } else {
        connection
            .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "GetNameOwner", &(bus,))
            .ok()
            .and_then(|reply| reply.body().deserialize::<String>().ok())
            .unwrap_or_else(|| bus.to_string())
    };
    let property = |name: &str| properties.get(name).map(|value| &**value);
    let word = |name: &str| property(name).and_then(text).filter(|s| !s.is_empty());
    let status = word("Status").unwrap_or_else(|| "Active".to_string());
    let alert = attention(&status);
    let theme_path = word("IconThemePath");
    let named = |name: Option<String>| {
        let name = name?;
        // An application's own icon folder comes first; then the usual themes.
        theme_path
            .as_deref()
            .and_then(|dir| crate::app_icons::resolve(&name, &[PathBuf::from(dir)]))
            .map(|path| path.to_string_lossy().into_owned())
            .or(Some(name))
    };
    let picture = |name: &str| property(name).map(pixmaps).and_then(|offered| pick_pixmap(&offered, PIXMAP_TARGET)).map(Arc::new);
    let (icon, pixmap) = if alert && (word("AttentionIconName").is_some() || property("AttentionIconPixmap").is_some()) {
        (named(word("AttentionIconName")), picture("AttentionIconPixmap"))
    } else {
        (named(word("IconName")), picture("IconPixmap"))
    };
    let menu_path = word("Menu").filter(|path| path != "/" && path.starts_with('/'));
    let menu = menu_path.as_deref().and_then(|menu| fetch_menu(connection, bus, menu));
    let item = Item {
        key: key.to_string(),
        bus: bus.to_string(),
        path: path.to_string(),
        owner,
        title: crate::notify::plain(&word("Title").or_else(|| word("Id")).unwrap_or_default()),
        tooltip: property("ToolTip").map(tooltip).unwrap_or_default(),
        status,
        icon,
        pixmap,
        generation,
        menu_path,
        item_is_menu: property("ItemIsMenu").and_then(boolean).unwrap_or(false),
        menu,
    };
    match items.iter_mut().find(|existing| existing.key == key) {
        Some(existing) => *existing = item,
        None => items.push(item),
    }
    true
}

fn fetch_menu(connection: &Connection, bus: &str, path: &str) -> Option<Vec<MenuNode>> {
    let reply = connection.call_method(Some(bus), path, Some(MENU), "GetLayout", &(0i32, -1i32, Vec::<&str>::new())).ok()?;
    let (_revision, root): (u32, OwnedValue) = reply.body().deserialize().ok()?;
    let mut budget = MAX_MENU_NODES;
    parse_node(&root, &mut budget).map(|root| root.children)
}

/// Calls a method of an item itself; a refusal is only logged, as items may not offer it.
fn call_item<B>(connection: &Connection, key: &str, method: &str, body: &B)
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    if let Some((bus, path)) = split_key(key) {
        if let Err(error) = connection.call_method(Some(bus), path, Some(ITEM), method, body) {
            eprintln!("des-ui: tray item {key}: {method}: {error}");
        }
    }
}

fn act(connection: &Connection, items: &mut [Item], command: Command) {
    match command {
        Command::Activate { key, x, y } => call_item(connection, &key, "Activate", &(x, y)),
        Command::Secondary { key, x, y } => call_item(connection, &key, "SecondaryActivate", &(x, y)),
        Command::Scroll { key, delta } => call_item(connection, &key, "Scroll", &(delta, "vertical")),
        Command::MenuOpened(key) => {
            if let Some(item) = items.iter_mut().find(|item| item.key == key) {
                if let Some(menu) = item.menu_path.clone() {
                    let _ = connection.call_method(Some(item.bus.as_str()), menu.as_str(), Some(MENU), "AboutToShow", &(0i32,));
                    item.menu = fetch_menu(connection, &item.bus, &menu);
                }
            }
        }
        Command::MenuClicked { key, id } => {
            if let Some(item) = items.iter().find(|item| item.key == key) {
                if let Some(menu) = &item.menu_path {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
                    let _ = connection.call_method(Some(item.bus.as_str()), menu.as_str(), Some(MENU), "Event", &(id, "clicked", Value::I32(0), now));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Drawing.

/// The tray's icons, in a row, for the status strip. Nothing when no application has one.
pub fn show(ui: &mut Ui, theme: &Theme, tray: &Tray) {
    let items: Vec<Item> = tray.shared.lock().unwrap().iter().filter(|item| visible(&item.status)).cloned().collect();
    if items.is_empty() {
        return;
    }
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 6.0;
    for item in items.iter().rev() {
        let (rect, response) = ui.allocate_exact_size(vec2(ICON + 4.0, ICON + 4.0), Sense::click());
        let response = response.on_hover_text(match (item.title.is_empty(), item.tooltip.is_empty()) {
            (_, false) => item.tooltip.clone(),
            (false, true) => item.title.clone(),
            (true, true) => "Application".to_string(),
        });
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &item.title));
        if response.hovered() {
            ui.painter().rect_filled(rect, 2.0, ui::TRACK);
        }
        let icon_rect = Rect::from_center_size(rect.center(), vec2(ICON, ICON));
        match &item.pixmap {
            Some(picture) => {
                let mut pixmaps = tray.pixmaps.borrow_mut();
                let fresh = pixmaps.get(&item.key).is_none_or(|(generation, _)| *generation != item.generation);
                if fresh {
                    let image = ColorImage::from_rgba_unmultiplied(picture.0, &picture.1);
                    let texture = ui.ctx().load_texture(format!("tray:{}", item.key), image, egui::TextureOptions::LINEAR);
                    pixmaps.insert(item.key.clone(), (item.generation, texture));
                }
                if let Some((_, texture)) = pixmaps.get(&item.key) {
                    let size = texture.size_vec2();
                    let scale = (ICON / size.x).min(ICON / size.y);
                    let target = Rect::from_center_size(icon_rect.center(), size * scale);
                    ui.painter().image(texture.id(), target, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
            }
            None => tray.icons.borrow_mut().paint(ui, icon_rect, item.icon.as_deref()),
        }
        if attention(&item.status) {
            ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(2.0, theme.main));
        }

        let at = response.interact_pointer_pos().unwrap_or(rect.center());
        let (x, y) = (at.x as i32, at.y as i32);
        let popup = ui.make_persistent_id(("tray-menu", &item.key));
        if response.clicked() {
            if item.item_is_menu && item.menu_path.is_some() {
                tray.send(Command::MenuOpened(item.key.clone()));
                ui.memory_mut(|memory| memory.toggle_popup(popup));
            } else {
                tray.send(Command::Activate { key: item.key.clone(), x, y });
            }
        }
        if response.middle_clicked() {
            tray.send(Command::Secondary { key: item.key.clone(), x, y });
        }
        if response.hovered() {
            let delta = ui.input(|input| input.raw_scroll_delta.y);
            if delta.abs() >= 1.0 {
                tray.send(Command::Scroll { key: item.key.clone(), delta: -delta.signum() as i32 * delta.abs().min(120.0) as i32 });
            }
        }
        if response.secondary_clicked() && item.menu_path.is_some() {
            tray.send(Command::MenuOpened(item.key.clone()));
        }
        if item.menu_path.is_some() {
            response.context_menu(|ui| {
                menu(ui, tray, item);
            });
            egui::popup::popup_above_or_below_widget(ui, popup, &response, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                if menu(ui, tray, item) {
                    ui.memory_mut(|memory| memory.close_popup());
                }
            });
        }
    }
    ui.spacing_mut().item_spacing.x = spacing;
}

/// Draws an item's menu; whether an entry was chosen.
fn menu(ui: &mut Ui, tray: &Tray, item: &Item) -> bool {
    ui.set_min_width(180.0);
    match &item.menu {
        Some(nodes) if nodes.iter().any(|node| node.visible) => entries(ui, tray, &item.key, nodes),
        _ => {
            ui.label(egui::RichText::new("No menu").font(ui::mono(12.0)).color(ui::SECONDARY));
            false
        }
    }
}

fn entries(ui: &mut Ui, tray: &Tray, key: &str, nodes: &[MenuNode]) -> bool {
    let mut chosen = false;
    for node in nodes.iter().filter(|node| node.visible) {
        if node.separator {
            ui.separator();
            continue;
        }
        let label = match node.toggle {
            Some(true) => format!("■ {}", node.label),
            Some(false) => format!("□ {}", node.label),
            None => node.label.clone(),
        };
        if !node.children.is_empty() {
            ui.add_enabled_ui(node.enabled, |ui| {
                ui.menu_button(label, |ui| {
                    if entries(ui, tray, key, &node.children) {
                        chosen = true;
                    }
                });
            });
        } else if ui.add_enabled(node.enabled, egui::Button::new(label)).clicked() {
            tray.send(Command::MenuClicked { key: key.to_string(), id: node.id });
            ui.close_menu();
            chosen = true;
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::StructureBuilder;

    #[test]
    fn items_register_by_name_path_or_both() {
        assert_eq!(item_address("org.kde.StatusNotifierItem-9-1", None), Some(("org.kde.StatusNotifierItem-9-1".into(), "/StatusNotifierItem".into())));
        assert_eq!(item_address("/org/ayatana/NotificationItem/nm", Some(":1.42")), Some((":1.42".into(), "/org/ayatana/NotificationItem/nm".into())));
        assert_eq!(item_address("/StatusNotifierItem", None), None);
        assert_eq!(item_address(":1.7/StatusNotifierItem", None), Some((":1.7".into(), "/StatusNotifierItem".into())));
        assert_eq!(item_address("", Some(":1.1")), None);
        assert_eq!(split_key(&key_of(":1.7", "/a/b")), Some((":1.7", "/a/b")));
    }

    #[test]
    fn the_watcher_forgets_what_a_vanished_name_registered() {
        let mut registry = Registry::default();
        assert!(registry.register(":1.5/StatusNotifierItem"));
        assert!(!registry.register(":1.5/StatusNotifierItem"));
        assert!(registry.register(":1.5/org/other"));
        assert!(registry.register(":1.50/StatusNotifierItem"));
        assert!(registry.register("org.app-1/StatusNotifierItem"));
        assert_eq!(registry.remove_owner(":1.5"), vec![":1.5/StatusNotifierItem".to_string(), ":1.5/org/other".to_string()]);
        assert_eq!(registry.items(), [":1.50/StatusNotifierItem".to_string(), "org.app-1/StatusNotifierItem".to_string()]);
        assert!(registry.remove("org.app-1/StatusNotifierItem"));
        assert!(!registry.remove("org.app-1/StatusNotifierItem"));
    }

    #[test]
    fn passive_items_hide_and_attention_is_noticed() {
        assert!(!visible("Passive"));
        assert!(visible("Active"));
        assert!(visible("NeedsAttention"));
        assert!(visible("Unknown"));
        assert!(attention("NeedsAttention"));
        assert!(!attention("Active"));
    }

    #[test]
    fn mnemonics_are_stripped_and_doubled_underscores_kept() {
        assert_eq!(strip_mnemonic("_Quit"), "Quit");
        assert_eq!(strip_mnemonic("Show _Window"), "Show Window");
        assert_eq!(strip_mnemonic("snake__case"), "snake_case");
        assert_eq!(strip_mnemonic("plain"), "plain");
    }

    #[test]
    fn the_nearest_pixmap_is_chosen_and_converted_to_rgba() {
        let pixel = [0x80, 0x10, 0x20, 0x30];
        let square = |side: i32| (side, side, pixel.repeat((side * side) as usize));
        let offered = vec![square(16), square(24), square(48), (22, 22, vec![0; 3])];
        let (size, rgba) = pick_pixmap(&offered, 22).unwrap();
        assert_eq!(size, [24, 24]);
        assert_eq!(&rgba[..4], &[0x10, 0x20, 0x30, 0x80]);
        assert_eq!(rgba.len(), 24 * 24 * 4);
        // A tie between 20 and 24 goes to the larger.
        let (size, _) = pick_pixmap(&[square(20), square(24)], 22).unwrap();
        assert_eq!(size, [24, 24]);
        assert!(pick_pixmap(&[(0, 0, vec![])], 22).is_none());
        assert!(pick_pixmap(&[(1024, 1, vec![0; 4096])], 22).is_none());
    }

    #[test]
    fn pixmaps_and_tooltips_are_read_from_their_values() {
        let picture = StructureBuilder::new().add_field(1i32).add_field(1i32).add_field(vec![1u8, 2, 3, 4]).build().unwrap();
        let array = Value::from(vec![Value::Structure(picture)]);
        assert_eq!(pixmaps(&array), vec![(1, 1, vec![1, 2, 3, 4])]);
        let tip = StructureBuilder::new()
            .add_field("icon")
            .add_field(Vec::<(i32, i32, Vec<u8>)>::new())
            .add_field("Nextcloud")
            .add_field("<b>Synced</b>")
            .build()
            .unwrap();
        assert_eq!(tooltip(&Value::Structure(tip)), "Nextcloud\nSynced");
    }

    fn entry(id: i32, properties: Vec<(&'static str, Value<'static>)>, children: Vec<Value<'static>>) -> Value<'static> {
        let properties: HashMap<&'static str, Value<'static>> = properties.into_iter().collect();
        let children = Value::from(children.into_iter().map(|child| Value::Value(Box::new(child))).collect::<Vec<_>>());
        Value::Structure(StructureBuilder::new().add_field(id).add_field(properties).append_field(children).build().unwrap())
    }

    #[test]
    fn a_dbusmenu_layout_becomes_a_tree() {
        let quit = entry(3, vec![("label", Value::from("_Quit"))], vec![]);
        let line = entry(4, vec![("type", Value::from("separator"))], vec![]);
        let mute = entry(5, vec![("label", Value::from("Mute")), ("toggle-type", Value::from("checkmark")), ("toggle-state", Value::from(1i32))], vec![]);
        let hidden = entry(6, vec![("label", Value::from("Secret")), ("visible", Value::from(false))], vec![]);
        let off = entry(7, vec![("label", Value::from("Off")), ("enabled", Value::from(false))], vec![]);
        let more = entry(2, vec![("label", Value::from("_More")), ("children-display", Value::from("submenu"))], vec![mute, off]);
        let root = entry(0, vec![], vec![more, line, hidden, quit]);
        let mut budget = 100;
        let tree = parse_node(&root, &mut budget).unwrap();
        assert_eq!(tree.children.len(), 4);
        let more = &tree.children[0];
        assert_eq!((more.id, more.label.as_str()), (2, "More"));
        assert_eq!(more.children[0].toggle, Some(true));
        assert!(!more.children[1].enabled);
        assert!(tree.children[1].separator);
        assert!(!tree.children[2].visible);
        assert_eq!(tree.children[3].label, "Quit");
        assert_eq!(tree.children[3].toggle, None);
    }

    #[test]
    fn huge_menus_stop_at_the_node_budget() {
        let children: Vec<Value<'static>> = (1..50).map(|id| entry(id, vec![("label", Value::from("x"))], vec![])).collect();
        let root = entry(0, vec![], children);
        let mut budget = 10;
        let tree = parse_node(&root, &mut budget).unwrap();
        assert_eq!(tree.children.len(), 9);
    }

    #[test]
    fn an_empty_tray_draws_nothing() {
        let ctx = egui::Context::default();
        let tray = Tray { shared: Arc::default(), events: Mutex::new(mpsc::channel().0), icons: RefCell::new(IconCache::new()), pixmaps: RefCell::default() };
        tray.shared.lock().unwrap().push(Item { key: ":1.1/StatusNotifierItem".into(), status: "Passive".into(), ..Default::default() });
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.cursor();
                show(ui, &Theme::load(crate::theme::DEFAULT_NAME), &tray);
                assert_eq!(ui.cursor(), before);
            });
        });
    }
}
