#![allow(unused_variables)]
#![allow(clippy::all)]
#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::needless_return,
    clippy::manual_range_contains,
    clippy::single_component_path_imports,
    clippy::len_zero,
    clippy::while_immutable_condition
)]

use core::ffi::{c_char, c_int, c_uint, c_void};
use libc;
use std::ffi::{CStr, CString};
use std::ptr;
use virtmcu_qom::chardev::{qemu_chr_be_write, Chardev, ChardevClass};
use virtmcu_qom::error::Error;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{declare_device_type, device_class, error_setg};
use zenoh::pubsub::Subscriber;
use zenoh::Session;
use zenoh::Wait;

#[repr(C)]
pub struct ChardevZenoh {
    pub parent: Chardev,
    pub rust_state: *mut ZenohChardevState,
}

pub struct ZenohChardevState {
    session: Session,
    chr: *mut Chardev,
    node_id: String,
    topic: String,
    subscriber: Option<Subscriber<()>>,
}

#[repr(C)]
struct ChardevZenohOptions {
    /* ChardevCommon */
    logfile: *mut c_char,
    has_logappend: bool,
    logappend: bool,
    has_logtimestamp: bool,
    logtimestamp: bool,
    _padding: [u8; 4],
    /* Own members */
    node: *mut c_char,
    router: *mut c_char,
    topic: *mut c_char,
}

#[repr(C)]
struct ChardevBackend {
    type_: c_int,
    padding: c_int, // Need padding after enum for union alignment if on 64-bit
    u: ChardevBackendUnion,
}

#[repr(C)]
union ChardevBackendUnion {
    data: *mut c_void,
}

const CHARDEV_BACKEND_KIND_ZENOH: c_int = 17;

extern "C" {
    pub fn qemu_opt_get(opts: *mut c_void, name: *const c_char) -> *const c_char;
    pub fn g_strdup(s: *const c_char) -> *mut c_char;
    pub fn g_malloc0(size: usize) -> *mut c_void;
    pub fn qemu_chr_parse_common(opts: *mut c_void, base: *mut c_void);
}

unsafe extern "C" fn zenoh_chr_write(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int {
    let s = &mut *(chr as *mut ChardevZenoh);
    if s.rust_state.is_null() {
        return 0;
    }
    zenoh_chardev_write_internal(&*s.rust_state, buf, len as usize) as c_int
}

unsafe extern "C" fn zenoh_chr_parse(
    opts: *mut c_void,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) {
    unsafe {
        libc::write(1, b"zenoh_chr_parse called\n".as_ptr() as *const c_void, 23);
    }
    let node = qemu_opt_get(opts, c"node".as_ptr());
    let router = qemu_opt_get(opts, c"router".as_ptr());
    let topic = qemu_opt_get(opts, c"topic".as_ptr());

    if node.is_null() {
        error_setg!(
            errp as *mut *mut Error,
            c"chardev: zenoh: 'node' is required".as_ptr()
        );
        return;
    }

    let zenoh_opts =
        g_malloc0(std::mem::size_of::<ChardevZenohOptions>()) as *mut ChardevZenohOptions;
    (*zenoh_opts).node = g_strdup(node);
    if !router.is_null() {
        (*zenoh_opts).router = g_strdup(router);
    }
    if !topic.is_null() {
        (*zenoh_opts).topic = g_strdup(topic);
    }

    let b = &mut *(backend as *mut ChardevBackend);
    b.type_ = CHARDEV_BACKEND_KIND_ZENOH;
    b.u.data = zenoh_opts as *mut c_void;

    qemu_chr_parse_common(opts, zenoh_opts as *mut c_void);
}

unsafe extern "C" fn zenoh_chr_open(
    chr: *mut Chardev,
    backend: *mut c_void,
    be_opened: *mut bool,
    errp: *mut *mut c_void,
) -> bool {
    let s = &mut *(chr as *mut ChardevZenoh);
    let b = &*(backend as *mut ChardevBackend);
    let opts = b.u.data as *mut ChardevZenohOptions;

    let node = CStr::from_ptr((*opts).node).to_string_lossy().into_owned();
    let router = if (*opts).router.is_null() {
        ptr::null()
    } else {
        (*opts).router as *const c_char
    };
    let topic = if (*opts).topic.is_null() {
        "sim/chardev".to_string()
    } else {
        CStr::from_ptr((*opts).topic).to_string_lossy().into_owned()
    };

    s.rust_state = zenoh_chardev_init_internal(chr, node, router, topic);
    if s.rust_state.is_null() {
        error_setg!(
            errp as *mut *mut Error,
            c"zenoh-chardev: failed to initialize Rust backend".as_ptr()
        );
        return false;
    }
    *be_opened = true;
    true
}

unsafe extern "C" fn zenoh_chr_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut ChardevZenoh);
    if !s.rust_state.is_null() {
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn char_zenoh_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let cc = &mut *(klass as *mut ChardevClass);
    cc.chr_parse = Some(zenoh_chr_parse);
    cc.chr_open = Some(zenoh_chr_open);
    cc.chr_write = Some(zenoh_chr_write);
}

static CHAR_ZENOH_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"chardev-zenoh".as_ptr(),
    parent: c"chardev".as_ptr(),
    instance_size: std::mem::size_of::<ChardevZenoh>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_chr_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(char_zenoh_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(char_zenoh_type_init, CHAR_ZENOH_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_chardev_init_internal(
    chr: *mut Chardev,
    node_id: String,
    router: *const c_char,
    topic: String,
) -> *mut ZenohChardevState {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    let full_topic = format!("{}/{}", topic, node_id);
    let chr_ptr = chr as usize;

    let subscriber = session
        .declare_subscriber(&full_topic)
        .callback(move |sample| {
            let chr = chr_ptr as *mut Chardev;
            let data = sample.payload().to_bytes();
            unsafe {
                virtmcu_qom::sync::virtmcu_bql_lock();
                qemu_chr_be_write(chr, data.as_ptr(), data.len());
                virtmcu_qom::sync::virtmcu_bql_unlock();
            }
        })
        .wait()
        .ok();

    Box::into_raw(Box::new(ZenohChardevState {
        session,
        chr,
        node_id,
        topic,
        subscriber,
    }))
}

fn zenoh_chardev_write_internal(state: &ZenohChardevState, buf: *const u8, len: usize) -> usize {
    let topic = format!("{}/{}", state.topic, state.node_id);
    let data = unsafe { std::slice::from_raw_parts(buf, len) };
    let _ = state.session.put(topic, data.to_vec()).wait();
    len
}
