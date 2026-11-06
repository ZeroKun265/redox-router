//! PipeWire backend: runs the PipeWire main loop on its own thread.
//!
//! - Registry events (nodes, ports, links) are read from global properties and
//!   sent to the UI as `Event`s.
//! - The UI sends `Command`s back through a `pw::channel`.
//! - Links we create via `link-factory` are NOT lingering, so PipeWire removes
//!   them automatically when RedoxRouter exits or crashes.

use crate::graph::{Command, Event, LinkInfo, NodeInfo, PortInfo};
use pipewire as pw;
use pw::device::Device;
use pw::link::Link;
use pw::node::Node;
use pw::proxy::{Listener, ProxyListener, ProxyT};
use pw::spa::param::ParamType;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, Value, ValueArray};
use std::io::Cursor;
use pw::properties::properties;
use pw::types::ObjectType;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;

pub type CommandSender = pw::channel::Sender<Command>;

// Numeric ids from PipeWire's spa/param/props.h, route.h, param.h and utils/type.h.
const PROP_VOLUME: u32 = 0x10003;
const PROP_MUTE: u32 = 0x10004;
const PROP_CHANNEL_VOLUMES: u32 = 0x10008;
const TYPE_OBJECT_PROPS: u32 = 0x40002;
const PARAM_PROPS: u32 = 2;
const ROUTE_DEVICE: u32 = 3;
const ROUTE_PROPS: u32 = 10;

type VolumeFields = (Option<f32>, Option<Vec<f32>>, Option<bool>);

/// Volume, channel volumes and mute out of the properties of a Props object.
fn props_fields(properties: Vec<Property>) -> VolumeFields {
    let (mut master, mut channels, mut mute) = (None, None, None);
    for p in properties {
        match (p.key, p.value) {
            (PROP_VOLUME, Value::Float(f)) => master = Some(f),
            (PROP_MUTE, Value::Bool(b)) => mute = Some(b),
            (PROP_CHANNEL_VOLUMES, Value::ValueArray(ValueArray::Float(v))) => channels = Some(v),
            _ => {}
        }
    }
    (master, channels, mute)
}

fn parse_object(pod: &Pod) -> Option<Object> {
    match PodDeserializer::deserialize_any_from(pod.as_bytes()).ok()?.1 {
        Value::Object(o) => Some(o),
        _ => None,
    }
}

/// A node's Props param.
fn parse_props(node_id: u32, pod: &Pod) -> Option<Event> {
    let (master, channels, mute) = props_fields(parse_object(pod)?.properties);
    if master.is_none() && channels.is_none() && mute.is_none() {
        return None;
    }
    Some(Event::Volume { node_id, master, channels, mute })
}

/// A device's Route param: the volume in its props, and which route device it is.
fn parse_route(device_id: u32, pod: &Pod) -> Option<Event> {
    let (mut device, mut fields) = (None, None);
    for p in parse_object(pod)?.properties {
        match (p.key, p.value) {
            (ROUTE_DEVICE, Value::Int(i)) => device = Some(i as u32),
            (ROUTE_PROPS, Value::Object(o)) => fields = Some(props_fields(o.properties)),
            _ => {}
        }
    }
    let (master, channels, mute) = fields?;
    if master.is_none() && channels.is_none() && mute.is_none() {
        return None;
    }
    Some(Event::RouteVolume { device_id, route_device: device?, master, channels, mute })
}

fn pod_bytes(obj: Object) -> Option<Vec<u8>> {
    let (cursor, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj)).ok()?;
    Some(cursor.into_inner())
}

/// Send a Props param to a node.
fn set_props(node: &Node, properties: Vec<Property>) {
    let obj = Object { type_: TYPE_OBJECT_PROPS, id: PARAM_PROPS, properties };
    if let Some(bytes) = pod_bytes(obj) {
        if let Some(pod) = Pod::from_bytes(&bytes) {
            node.set_param(ParamType::Props, 0, pod);
        }
    }
}

fn volume_props(linear: f32, channels: usize) -> Vec<Property> {
    if channels > 0 {
        vec![Property::new(
            PROP_CHANNEL_VOLUMES,
            Value::ValueArray(ValueArray::Float(vec![linear; channels])),
        )]
    } else {
        vec![Property::new(PROP_VOLUME, Value::Float(linear))]
    }
}

/// Start the PipeWire thread. `ctx` is used to wake the UI when events arrive.
/// Returns the sender for commands to the thread.
pub fn spawn(tx: Sender<Event>, ctx: egui::Context) -> CommandSender {
    pw::init();
    let (cmd_tx, cmd_rx) = pw::channel::channel::<Command>();
    std::thread::spawn(move || {
        if let Err(e) = run(&tx, &ctx, cmd_rx) {
            let _ = tx.send(Event::Error(e.to_string()));
            ctx.request_repaint();
        }
    });
    cmd_tx
}

fn debug_enabled() -> bool {
    static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("REDOX_DEBUG").is_some() {
            ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

macro_rules! rlog {
    ($($arg:tt)*) => {
        if debug_enabled() {
            eprintln!("[REDOX] {}", format_args!($($arg)*));
        }
    };
}

struct OwnedLink {
    link: Link,
    global_id: Rc<RefCell<Option<u32>>>,
    _listener: ProxyListener,
}

struct OwnedSink {
    _node: Node,
    global_id: Rc<RefCell<Option<u32>>>,
    _listener: ProxyListener,
}

fn run(
    tx: &Sender<Event>,
    ctx: &egui::Context,
    cmd_rx: pw::channel::Receiver<Command>,
) -> Result<(), pw::Error> {
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;

    // Quit the loop only if the connection to the daemon dies.
    let main_loop_weak = main_loop.downgrade();
    let (tx_err, ctx_err) = (tx.clone(), ctx.clone());
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _seq, res, message| {
            rlog!("core error id={id} res={res}: {message}");
            // -ENOENT (-2) happens benignly when an object disappears right as we bind or destroy it.
            if res != -2 {
                let _ = tx_err.send(Event::Error(format!("pipewire error {id}/{res}: {message}")));
                ctx_err.request_repaint();
            }
            // Quit the loop only if the connection is dead (EPIPE = -32, ECONNRESET = -104).
            // Do NOT quit on method/resource errors (e.g. -ENOENT = -2 for unknown resource).
            if id == 0 && (res == -32 || res == -104) {
                rlog!("fatal connection error, quitting main loop");
                if let Some(main_loop) = main_loop_weak.upgrade() {
                    main_loop.quit();
                }
            }
        })
        .register();

    let registry = core.get_registry_rc()?;

    // Audio nodes we watch (proxies and their listeners must stay alive).
    let nodes: Rc<RefCell<HashMap<u32, Node>>> = Rc::new(RefCell::new(HashMap::new()));
    let devices: Rc<RefCell<HashMap<u32, Device>>> = Rc::new(RefCell::new(HashMap::new()));
    let listeners: Rc<RefCell<HashMap<u32, Box<dyn Listener>>>> =
        Rc::new(RefCell::new(HashMap::new()));

    // Commands from the UI. We keep the proxies and listeners of links we created, keyed by
    // (out_port, in_port).
    let owned_links: Rc<RefCell<HashMap<(u32, u32), OwnedLink>>> =
        Rc::new(RefCell::new(HashMap::new()));
    // Virtual sinks created by RedoxRouter (kept alive while running, destroyed on exit).
    let owned_sinks: Rc<RefCell<Vec<OwnedSink>>> = Rc::new(RefCell::new(Vec::new()));
    let (core_cmd, registry_cmd) = (core.clone(), registry.clone());
    let (tx_cmd, ctx_cmd) = (tx.clone(), ctx.clone());
    let nodes_cmd = nodes.clone();
    let owned_links_cmd = owned_links.clone();
    let owned_sinks_cmd = owned_sinks.clone();
    let _cmd_receiver = cmd_rx.attach(main_loop.loop_(), move |cmd| {
        let notify = |ev: Event| {
            let _ = tx_cmd.send(ev);
            ctx_cmd.request_repaint();
        };
        let create = |out_node: u32, out_port: u32, in_node: u32, in_port: u32| {
            let key = (out_port, in_port);
            if owned_links_cmd.borrow().contains_key(&key) {
                rlog!("create link {:?}: already in owned_links", key);
                return;
            }
            rlog!("create link out_node={out_node} out_port={out_port} in_node={in_node} in_port={in_port}");
            let props = properties! {
                "link.output.node" => out_node.to_string(),
                "link.output.port" => out_port.to_string(),
                "link.input.node" => in_node.to_string(),
                "link.input.port" => in_port.to_string(),
                "object.linger" => "false"
            };
            match core_cmd.create_object::<Link>("link-factory", &props) {
                Ok(link) => {
                    let gid = Rc::new(RefCell::new(None));
                    let gid_bound = gid.clone();
                    let owned_links_weak = Rc::downgrade(&owned_links_cmd);
                    let key_clone = key;

                    let listener = link
                        .upcast_ref()
                        .add_listener_local()
                        .bound(move |id| {
                            rlog!("owned link {:?} bound to global id {id}", key_clone);
                            *gid_bound.borrow_mut() = Some(id);
                        })
                        .removed(move || {
                            rlog!("owned link {:?} proxy removed by server", key_clone);
                            if let Some(links) = owned_links_weak.upgrade() {
                                links.borrow_mut().remove(&key_clone);
                            }
                        })
                        .register();

                    owned_links_cmd.borrow_mut().insert(
                        key,
                        OwnedLink {
                            link,
                            global_id: gid,
                            _listener: listener,
                        },
                    );
                    notify(Event::OwnedAdded { out_port, in_port });
                }
                Err(e) => {
                    rlog!("create link failed: {e}");
                    notify(Event::Error(format!("could not create link: {e}")));
                }
            }
        };
        match cmd {
            Command::CreateLink { out_node, out_port, in_node, in_port } => {
                create(out_node, out_port, in_node, in_port);
            }
            Command::TakeOver { link_id, out_node, out_port, in_node, in_port } => {
                rlog!("take over link_id={link_id}");
                let _ = registry_cmd.destroy_global(link_id);
                create(out_node, out_port, in_node, in_port);
            }
            Command::SetVolume { node_id, linear, channels } => {
                if let Some(node) = nodes_cmd.borrow().get(&node_id) {
                    let mut props = volume_props(linear, channels);
                    if channels > 0 {
                        props.push(Property::new(PROP_VOLUME, Value::Float(1.0)));
                    }
                    set_props(node, props);
                }
            }
            Command::SetMute { node_id, mute } => {
                if let Some(node) = nodes_cmd.borrow().get(&node_id) {
                    set_props(node, vec![Property::new(PROP_MUTE, Value::Bool(mute))]);
                }
            }
            Command::DestroyLink { link_id, out_port, in_port } => {
                rlog!("command DestroyLink link_id={:?} out_port={} in_port={}", link_id, out_port, in_port);
                let removed = owned_links_cmd.borrow_mut().remove(&(out_port, in_port));
                let id_to_destroy =
                    link_id.or_else(|| removed.as_ref().and_then(|ol| *ol.global_id.borrow()));
                if let Some(id) = id_to_destroy {
                    rlog!("destroying link global id {id} via registry");
                    let _ = registry_cmd.destroy_global(id);
                } else if let Some(ol) = removed {
                    rlog!("destroying unbound link proxy via core");
                    let _ = core_cmd.destroy_object(ol.link);
                }
                notify(Event::OwnedRemoved { out_port, in_port });
            }
            Command::CreateSink { name, description } => {
                rlog!("command CreateSink name={name} description={description}");
                let props = properties! {
                    "factory.name" => "support.null-audio-sink",
                    "node.name" => name,
                    "node.description" => description,
                    "media.class" => "Audio/Sink",
                    "audio.position" => "FL,FR",
                    "object.linger" => "false",
                    "redox.owned" => "true"
                };
                match core_cmd.create_object::<Node>("adapter", &props) {
                    Ok(node) => {
                        rlog!("created virtual sink node proxy, waiting for global bind");
                        let gid = Rc::new(RefCell::new(None));
                        let gid_bound = gid.clone();
                        let tx_sink = tx_cmd.clone();
                        let ctx_sink = ctx_cmd.clone();

                        let listener = node
                            .upcast_ref()
                            .add_listener_local()
                            .bound(move |id| {
                                rlog!("owned sink bound to global id {id}");
                                *gid_bound.borrow_mut() = Some(id);
                                let _ = tx_sink.send(Event::OwnedNodeAdded(id));
                                ctx_sink.request_repaint();
                            })
                            .removed(move || {
                                rlog!("owned sink proxy removed by server");
                            })
                            .register();

                        owned_sinks_cmd.borrow_mut().push(OwnedSink {
                            _node: node,
                            global_id: gid,
                            _listener: listener,
                        });
                    }
                    Err(e) => {
                        rlog!("create sink failed: {e}");
                        notify(Event::Error(format!("could not create sink: {e}")));
                    }
                }
            }
            Command::CreateSource { name, description } => {
                rlog!("command CreateSource name={name} description={description}");
                let props = properties! {
                    "factory.name" => "support.null-audio-sink",
                    "node.name" => name,
                    "node.description" => description,
                    "media.class" => "Audio/Source/Virtual",
                    "audio.position" => "FL,FR",
                    "object.linger" => "false",
                    "redox.owned" => "true"
                };
                match core_cmd.create_object::<Node>("adapter", &props) {
                    Ok(node) => {
                        rlog!("created virtual source node proxy, waiting for global bind");
                        let gid = Rc::new(RefCell::new(None));
                        let gid_bound = gid.clone();
                        let tx_sink = tx_cmd.clone();
                        let ctx_sink = ctx_cmd.clone();

                        let listener = node
                            .upcast_ref()
                            .add_listener_local()
                            .bound(move |id| {
                                rlog!("owned source bound to global id {id}");
                                *gid_bound.borrow_mut() = Some(id);
                                let _ = tx_sink.send(Event::OwnedNodeAdded(id));
                                ctx_sink.request_repaint();
                            })
                            .removed(move || {
                                rlog!("owned source proxy removed by server");
                            })
                            .register();

                        owned_sinks_cmd.borrow_mut().push(OwnedSink {
                            _node: node,
                            global_id: gid,
                            _listener: listener,
                        });
                    }
                    Err(e) => {
                        rlog!("create virtual mic failed: {e}");
                        notify(Event::Error(format!("could not create virtual mic: {e}")));
                    }
                }
            }
            Command::DestroySink { node_id } => {
                rlog!("command DestroySink node_id={node_id}");
                owned_sinks_cmd.borrow_mut().retain(|os| {
                    os.global_id.borrow().map_or(true, |gid| gid != node_id)
                });
                let _ = registry_cmd.destroy_global(node_id);
            }
        }
    });

    let (tx_add, ctx_add) = (tx.clone(), ctx.clone());
    let (tx_rm, ctx_rm) = (tx.clone(), ctx.clone());
    let registry_weak = registry.downgrade();
    let (nodes_add, listeners_add) = (nodes.clone(), listeners.clone());
    let (nodes_rm, listeners_rm) = (nodes.clone(), listeners.clone());
    let (devices_add, devices_rm) = (devices.clone(), devices.clone());
    let owned_links_rm = owned_links.clone();
    let owned_sinks_rm = owned_sinks.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |obj| {
            let Some(props) = obj.props else { return };
            let get = |key: &str| props.get(key).map(str::to_string);
            let event = match obj.type_ {
                ObjectType::Node => Some(Event::NodeAdded(NodeInfo::from_props(obj.id, &get))),
                ObjectType::Port => PortInfo::from_props(obj.id, &get).map(Event::PortAdded),
                ObjectType::Link => LinkInfo::from_props(obj.id, &get).map(Event::LinkAdded),
                _ => None,
            };
            if let Some(ref ev) = event {
                if let Event::LinkAdded(ref l) = ev {
                    rlog!(
                        "global link added: id={} out_node={} out_port={} in_node={} in_port={}",
                        l.id, l.out_node, l.out_port, l.in_node, l.in_port
                    );
                }
            }
            if let Some(event) = event {
                let _ = tx_add.send(event);
                ctx_add.request_repaint();
            }

            // Audio nodes: watch their Props for volume and mute changes.
            let is_audio = obj.type_ == ObjectType::Node
                && get("media.class").is_some_and(|c| c.contains("Audio"));
            if is_audio {
                if let Some(reg) = registry_weak.upgrade() {
                    let bound: Result<Node, _> = reg.bind(obj);
                    match bound {
                        Ok(node) => {
                            let id = obj.id;
                            let (tx_p, ctx_p) = (tx_add.clone(), ctx_add.clone());
                            let listener = node
                                .add_listener_local()
                                .param(move |_seq, _id, _index, _next, param| {
                                    if let Some(ev) = param.and_then(|p| parse_props(id, p)) {
                                        let _ = tx_p.send(ev);
                                        ctx_p.request_repaint();
                                    }
                                })
                                .register();
                            node.subscribe_params(&[ParamType::Props]);
                            nodes_add.borrow_mut().insert(id, node);
                            listeners_add.borrow_mut().insert(id, Box::new(listener));
                        }
                        Err(e) => {
                            let _ = tx_add.send(Event::Error(format!("cannot watch node: {e}")));
                        }
                    }
                }
            }

            // Sound cards: watch their routes, where the hardware volume lives.
            let is_card = obj.type_ == ObjectType::Device
                && get("media.class").is_some_and(|c| c == "Audio/Device");
            if is_card {
                if let Some(reg) = registry_weak.upgrade() {
                    let bound: Result<Device, _> = reg.bind(obj);
                    match bound {
                        Ok(dev) => {
                            let id = obj.id;
                            let (tx_p, ctx_p) = (tx_add.clone(), ctx_add.clone());
                            let listener = dev
                                .add_listener_local()
                                .param(move |_seq, _id, _index, _next, param| {
                                    if let Some(ev) = param.and_then(|p| parse_route(id, p)) {
                                        let _ = tx_p.send(ev);
                                        ctx_p.request_repaint();
                                    }
                                })
                                .register();
                            dev.subscribe_params(&[ParamType::Route]);
                            devices_add.borrow_mut().insert(id, dev);
                            listeners_add.borrow_mut().insert(id, Box::new(listener));
                        }
                        Err(e) => {
                            let _ = tx_add.send(Event::Error(format!("cannot watch device: {e}")));
                        }
                    }
                }
            }
        })
        .global_remove(move |id| {
            rlog!("global_remove id={id}");
            listeners_rm.borrow_mut().remove(&id);
            devices_rm.borrow_mut().remove(&id);
            nodes_rm.borrow_mut().remove(&id);
            owned_links_rm.borrow_mut().retain(|k, ol| {
                if ol.global_id.borrow().map_or(false, |gid| gid == id) {
                    rlog!("owned_links dropping entry {:?} matching global id {id}", k);
                    false
                } else {
                    true
                }
            });
            owned_sinks_rm.borrow_mut().retain(|os| {
                if os.global_id.borrow().map_or(false, |gid| gid == id) {
                    rlog!("owned_sinks dropping entry matching global id {id}");
                    false
                } else {
                    true
                }
            });
            let _ = tx_rm.send(Event::Removed(id));
            ctx_rm.request_repaint();
        })
        .register();

    main_loop.run();
    Ok(())
}
