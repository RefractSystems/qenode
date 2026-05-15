use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Data, DeriveInput, Fields, ItemStruct, LitStr, Type};

#[proc_macro_derive(MmioDevice)]
pub fn derive_mmio_device(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let mut state_field = quote! { rust_state };

    if let Data::Struct(ref data) = input.data {
        if let Fields::Named(ref fields) = data.fields {
            for field in &fields.named {
                if field.ident.as_ref().unwrap() == "state" {
                    state_field = quote! { state };
                    break;
                }
            }
        }
    }

    let qemu_struct = name;
    let ops_name = format_ident!("{}_OPS", name.to_string().to_uppercase());
    let read_fn = format_ident!("{}_read_shim", name.to_string().to_lowercase());
    let write_fn = format_ident!("{}_write_shim", name.to_string().to_lowercase());

    let expanded = quote! {
        const BQL_YIELD_TIMEOUT_MS: u32 = 100;
        const MAX_ACCESS_SIZE: u32 = 8;

        unsafe extern "C" fn #read_fn(opaque: *mut core::ffi::c_void, offset: u64, size: core::ffi::c_uint) -> u64 {
            let s = unsafe { &mut *(opaque as *mut #qemu_struct) };
            let state_ptr = s.#state_field;
            if state_ptr.is_null() {
                return 0;
            }
            let state = unsafe { &*state_ptr };

            let mut res = virtmcu_qom::device::MmioDevice::read(state, offset, size as u32);
            match res {
                virtmcu_qom::device::MmioResult::Ready(val) => val,
                virtmcu_qom::device::MmioResult::Wait { mut condition, mut ready_val, mut fallback_val } => {
                    if virtmcu_qom::icount::icount_enabled() {
                        if condition() {
                            ready_val()
                        } else {
                            {
                                let _unlock = virtmcu_qom::sync::Bql::temporary_unlock();
                                // virtmcu-allow: yield reasoning="Required to advance icount"
                                std::thread::yield_now();
                            }
                            fallback_val()
                        }
                    } else {
                        let cond = virtmcu_qom::device::MmioDevice::condvar(state);
                        let mutex = virtmcu_qom::device::MmioDevice::wait_mutex(state);
                        let mut guard = mutex.lock();
                        loop {
                            if condition() { return ready_val(); }
                            let (g, _) = cond.wait_yielding_bql(guard, BQL_YIELD_TIMEOUT_MS);
                            guard = g;
                        }
                    }
                }
            }
        }

        unsafe extern "C" fn #write_fn(opaque: *mut core::ffi::c_void, offset: u64, value: u64, size: core::ffi::c_uint) {
            let s = unsafe { &mut *(opaque as *mut #qemu_struct) };
            let state_ptr = s.#state_field;
            if state_ptr.is_null() {
                return;
            }
            let state = unsafe { &*state_ptr };
            virtmcu_qom::device::MmioDevice::write(state, offset, value, size as u32);
        }

        pub static #ops_name: virtmcu_qom::memory::MemoryRegionOps = virtmcu_qom::memory::MemoryRegionOps {
            read: Some(#read_fn),
            write: Some(#write_fn),
            read_with_attrs: core::ptr::null(),
            write_with_attrs: core::ptr::null(),
            endianness: virtmcu_qom::memory::DEVICE_LITTLE_ENDIAN,
            _padding1: [0; 4],
            valid: virtmcu_qom::memory::MemoryRegionValidRange {
                min_access_size: 1,
                max_access_size: MAX_ACCESS_SIZE,
                unaligned: false,
                _padding: [0; 7],
                accepts: core::ptr::null(),
            },
            impl_: virtmcu_qom::memory::MemoryRegionImplRange {
                min_access_size: 1,
                max_access_size: MAX_ACCESS_SIZE,
                unaligned: false,
                _padding: [0; 7],
            },
        };
    };

    TokenStream::from(expanded)
}

/// RFC-0023 Phase 3 & 4: Declarative QOM Device and Safe Lifecycles.
#[proc_macro_attribute]
pub fn qom_device(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut qom_name = String::new();
    let mut qom_parent = String::from("sys-bus-device");
    let mut class_init_custom = None;

    let attr_parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("name") {
            let value = meta.value()?;
            let s: LitStr = value.parse()?;
            qom_name = s.value();
        } else if meta.path.is_ident("parent") {
            let value = meta.value()?;
            let s: LitStr = value.parse()?;
            qom_parent = s.value();
        } else if meta.path.is_ident("class_init_custom") {
            let value = meta.value()?;
            let s: LitStr = value.parse()?;
            class_init_custom = Some(s.value());
        }
        Ok(())
    });
    parse_macro_input!(attr with attr_parser);

    let mut input = parse_macro_input!(item as ItemStruct);
    let name = &input.ident;
    let name_str = name.to_string();

    let mut props = Vec::new();
    let mut links = Vec::new();
    let mut state_field_type = None;
    let mut state_field_name = None;

    if let Fields::Named(ref mut fields) = input.fields {
        for field in &mut fields.named {
            let mut is_prop = false;
            let mut is_link = false;
            let mut is_state = false;
            let mut link_target = String::new();
            let prop_name = field.ident.as_ref().unwrap().to_string().replace('_', "-");
            let default_val = quote! { 0 };

            field.attrs.retain(|attr| {
                if attr.path().is_ident("qom_property") {
                    is_prop = true;
                    false
                } else if attr.path().is_ident("qom_link") {
                    is_link = true;
                    let _ = attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("target") {
                            let value = meta.value()?;
                            let s: syn::LitStr = value.parse()?;
                            link_target = s.value();
                        }
                        Ok(())
                    });
                    false
                } else if attr.path().is_ident("qom_state") {
                    is_state = true;
                    false
                } else {
                    true
                }
            });

            let field_ident = field.ident.as_ref().unwrap();
            let prop_name_c = format!("{}\0", prop_name);
            let prop_name_lit = syn::LitByteStr::new(prop_name_c.as_bytes(), field_ident.span());

            if is_state {
                state_field_type = Some(field.ty.clone());
                state_field_name = Some(field_ident.clone());
                field.ty = syn::parse_quote! { *mut #state_field_type };
            }

            if is_prop {
                let field_ty = &field.ty;
                let define_macro;
                let mut needs_default = true;

                if is_type(field_ty, "u64") {
                    define_macro = quote! { virtmcu_qom::define_prop_uint64 };
                } else if is_type(field_ty, "u32") {
                    define_macro = quote! { virtmcu_qom::define_prop_uint32 };
                } else if is_type(field_ty, "bool") {
                    define_macro = quote! { virtmcu_qom::define_prop_bool };
                } else if is_type(field_ty, "*mut c_char")
                    || is_type(field_ty, "c_char")
                    || is_type(field_ty, "QomString")
                    || is_type(field_ty, "virtmcu_qom::qom::QomString")
                {
                    define_macro = quote! { virtmcu_qom::define_prop_string };
                    needs_default = false; // define_prop_string only takes 3 arguments
                } else {
                    define_macro = quote! { virtmcu_qom::define_prop_uint64 };
                };

                if needs_default {
                    props.push(quote! {
                        #define_macro!(#prop_name_lit.as_ptr() as *const core::ffi::c_char, #name, #field_ident, #default_val)
                    });
                } else {
                    props.push(quote! {
                        #define_macro!(#prop_name_lit.as_ptr() as *const core::ffi::c_char, #name, #field_ident)
                    });
                }
            }

            if is_link {
                let link_target_c = format!("{}\0", link_target);
                let target_lit = syn::LitByteStr::new(link_target_c.as_bytes(), field_ident.span());
                links.push(quote! {
                    virtmcu_qom::qom::object_class_property_add_link(
                        klass,
                        #prop_name_lit.as_ptr() as *const core::ffi::c_char,
                        #target_lit.as_ptr() as *const core::ffi::c_char,
                        core::mem::offset_of!(#name, #field_ident) as isize,
                        Some(allow_set_link),
                        virtmcu_qom::qom::OBJ_PROP_LINK_STRONG,
                    );
                });
            }
        }
    }

    let state_ty = state_field_type.expect("Missing #[qom_state] field");
    let state_field = state_field_name.expect("Missing #[qom_state] field");
    let prop_array_name = format_ident!("{}_PROPERTIES", name.to_string().to_uppercase());
    let prop_count = props.len();

    let realize_fn = format_ident!("{}_realize", name_str.to_lowercase());
    let finalize_fn = format_ident!("{}_finalize", name_str.to_lowercase());
    let init_fn = format_ident!("{}_instance_init", name_str.to_lowercase());
    let class_init_fn = format_ident!("{}_class_init", name_str.to_lowercase());
    let type_info_name = format_ident!("{}_TYPE_INFO", name.to_string().to_uppercase());
    let mmio_ops_name = format_ident!("{}_OPS", name_str.to_uppercase());

    let qom_name_lit = LitStr::new(&qom_name, name.span());
    let qom_name_c = format!("{}\0", qom_name);
    let qom_name_c_lit = syn::LitByteStr::new(qom_name_c.as_bytes(), name.span());
    let qom_parent_c = format!("{}\0", qom_parent);
    let qom_parent_lit = syn::LitByteStr::new(qom_parent_c.as_bytes(), name.span());

    let custom_init_call = if let Some(custom_init) = class_init_custom {
        let custom_fn = format_ident!("{}", custom_init);
        quote! { #custom_fn(klass, _data); }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #input

        #[cfg(test)]
        impl #name {
            /// Creates a zero-initialized mock instance for testing.
            /// This encapsulates unsafe zero-initialization of QEMU FFI types.
            pub fn new_mock() -> Self {
                // SAFETY: This is strictly for unit testing where full QEMU initialization is not required.
                unsafe { core::mem::zeroed() }
            }
        }

        #[used]
        static #prop_array_name: [virtmcu_qom::qom::Property; #prop_count] = [
            #(#props),*
        ];

        unsafe extern "C" fn allow_set_link(
            _obj: *mut virtmcu_qom::qom::Object,
            _name: *const core::ffi::c_char,
            _val: *mut virtmcu_qom::qom::Object,
            _errp: *mut *mut virtmcu_qom::error::Error,
        ) {}

        unsafe extern "C" fn #init_fn(obj: *mut virtmcu_qom::qom::Object) {
            let s = unsafe { &mut *(obj as *mut #name) };
            s.#state_field = core::ptr::null_mut();
        }

        unsafe extern "C" fn #finalize_fn(obj: *mut virtmcu_qom::qom::Object) {
            let s = unsafe { &mut *(obj as *mut #name) };
            if !s.#state_field.is_null() {
                unsafe {
                    drop(Box::from_raw(s.#state_field));
                }
                s.#state_field = core::ptr::null_mut();
            }
        }

        unsafe extern "C" fn #realize_fn(dev: *mut core::ffi::c_void, _errp: *mut *mut core::ffi::c_void) {
            const DEFAULT_MMIO_REGION_SIZE: u64 = 0x1000;
            let s = unsafe { &mut *(dev as *mut #name) };
            if !s.#state_field.is_null() {
                return;
            }

            let mut state = Box::new(<#state_ty as virtmcu_qom::device::PeripheralState>::new(s));

            if let Err(e) = virtmcu_qom::device::Peripheral::realize(&mut *state) {
                virtmcu_qom::sim_err!("{}: realization failed: {}", #qom_name_lit, e);
            }

            s.#state_field = Box::into_raw(state);

            virtmcu_qom::memory::memory_region_init_io(
                &raw mut s.iomem,
                dev as *mut virtmcu_qom::qom::Object,
                &raw const #mmio_ops_name,
                dev,
                #qom_name_c_lit.as_ptr() as *const core::ffi::c_char,
                DEFAULT_MMIO_REGION_SIZE,
            );
            virtmcu_qom::qdev::sysbus_init_mmio(dev as *mut virtmcu_qom::qdev::SysBusDevice, &raw mut s.iomem);
        }

        unsafe extern "C" fn #class_init_fn(klass: *mut virtmcu_qom::qom::ObjectClass, _data: *const core::ffi::c_void) {
            let dc = virtmcu_qom::device_class!(klass);
            (*dc).realize = Some(#realize_fn);
            (*dc).user_creatable = true;
            virtmcu_qom::qdev::device_class_set_props_n(
                dc,
                #prop_array_name.as_ptr(),
                #prop_count,
            );
            unsafe {
                #(#links)*
            }
            #custom_init_call
        }

        #[used]
        pub static #type_info_name: virtmcu_qom::qom::TypeInfo = virtmcu_qom::qom::TypeInfo {
            name: #qom_name_c_lit.as_ptr() as *const core::ffi::c_char,
            parent: #qom_parent_lit.as_ptr() as *const core::ffi::c_char,
            instance_size: core::mem::size_of::<#name>(),
            instance_align: 0,
            instance_init: Some(#init_fn),
            instance_post_init: None,
            instance_finalize: Some(#finalize_fn),
            abstract_: false,
            class_size: 0,
            class_init: Some(#class_init_fn),
            class_base_init: None,
            class_data: core::ptr::null(),
            interfaces: core::ptr::null(),
        };
    };

    TokenStream::from(expanded)
}

fn is_type(ty: &Type, name: &str) -> bool {
    if let Type::Path(ref p) = ty {
        if let Some(segment) = p.path.segments.last() {
            return segment.ident == name;
        }
    } else if let Type::Ptr(ref ptr) = ty {
        // Simple check for *mut c_char
        if name == "*mut c_char" {
            if let Type::Path(ref p) = *ptr.elem {
                if let Some(segment) = p.path.segments.last() {
                    return segment.ident == "c_char";
                }
            }
        }
    }
    false
}
