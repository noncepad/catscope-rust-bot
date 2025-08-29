//   * stubs
//   * ownership: Borrowing { duplicate_if_necessary: true }
//   * pub-export-macro
//   * generate_unused_types
impl exports::catscope::witbot::updater::Guest for Stub {
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn transactionv1(
        signature: _rt::Vec<u8>,
        slot: u64,
        status: u8,
        txresult: u32,
    ) -> () {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn accountv1(
        id: u64,
        slot: u64,
        lamports: u64,
        rent: u64,
        owner: u64,
        datasize: u32,
    ) -> () {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn tokenv1(
        id: u64,
        slot: u64,
        lamports: u64,
        mint: u64,
        owner: u64,
        balance: u64,
    ) -> () {
        unreachable!()
    }
    #[allow(unused_variables)]
    #[allow(async_fn_in_trait)]
    fn slot(slot: u64, status: u8) -> () {
        unreachable!()
    }
}
impl exports::wasi::cli::run::Guest for Stub {
    #[allow(unused_variables)]
    /// Run the program.
    #[allow(async_fn_in_trait)]
    fn run() -> Result<(), ()> {
        unreachable!()
    }
}
#[rustfmt::skip]
#[allow(dead_code, clippy::all)]
pub mod catscope {
    pub mod witbot {
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod transactionprocessor {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum ErrorCode {
                Unknown,
                AccessDenied,
                NotSupported,
                InvalidArgument,
                OutOfMemory,
                Timeout,
            }
            impl ErrorCode {
                pub fn name(&self) -> &'static str {
                    match self {
                        ErrorCode::Unknown => "unknown",
                        ErrorCode::AccessDenied => "access-denied",
                        ErrorCode::NotSupported => "not-supported",
                        ErrorCode::InvalidArgument => "invalid-argument",
                        ErrorCode::OutOfMemory => "out-of-memory",
                        ErrorCode::Timeout => "timeout",
                    }
                }
                pub fn message(&self) -> &'static str {
                    match self {
                        ErrorCode::Unknown => "",
                        ErrorCode::AccessDenied => "",
                        ErrorCode::NotSupported => "",
                        ErrorCode::InvalidArgument => "",
                        ErrorCode::OutOfMemory => "",
                        ErrorCode::Timeout => "",
                    }
                }
            }
            impl ::core::fmt::Debug for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("ErrorCode")
                        .field("code", &(*self as i32))
                        .field("name", &self.name())
                        .field("message", &self.message())
                        .finish()
                }
            }
            impl ::core::fmt::Display for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    write!(f, "{} (error {})", self.name(), * self as i32)
                }
            }
            impl std::error::Error for ErrorCode {}
            impl ErrorCode {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> ErrorCode {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => ErrorCode::Unknown,
                        1 => ErrorCode::AccessDenied,
                        2 => ErrorCode::NotSupported,
                        3 => ErrorCode::InvalidArgument,
                        4 => ErrorCode::OutOfMemory,
                        5 => ErrorCode::Timeout,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            /// on success, return the slot
            #[allow(async_fn_in_trait)]
            pub fn send(
                signature: &[u8],
                txdata: &[u8],
            ) -> wit_bindgen::rt::async_support::FutureReader<Result<u64, ErrorCode>> {
                unsafe {
                    let vec0 = signature;
                    let ptr0 = vec0.as_ptr().cast::<u8>();
                    let len0 = vec0.len();
                    let vec1 = txdata;
                    let ptr1 = vec1.as_ptr().cast::<u8>();
                    let len1 = vec1.len();
                    #[cfg(target_arch = "wasm32")]
                    #[link(
                        wasm_import_module = "catscope:witbot/transactionprocessor@0.1.0"
                    )]
                    unsafe extern "C" {
                        #[link_name = "send"]
                        fn wit_import2(
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                            _: usize,
                        ) -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import2(
                        _: *mut u8,
                        _: usize,
                        _: *mut u8,
                        _: usize,
                    ) -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import2(ptr0.cast_mut(), len0, ptr1.cast_mut(), len1);
                    wit_bindgen::rt::async_support::FutureReader::new(
                        ret as u32,
                        &super::super::super::wit_future::vtable0::VTABLE,
                    )
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn keygen() -> Result<_rt::Vec<u8>, ErrorCode> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 3 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 3
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(
                        wasm_import_module = "catscope:witbot/transactionprocessor@0.1.0"
                    )]
                    unsafe extern "C" {
                        #[link_name = "keygen"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result7 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0
                                    .add(::core::mem::size_of::<*const u8>())
                                    .cast::<*mut u8>();
                                let l4 = *ptr0
                                    .add(2 * ::core::mem::size_of::<*const u8>())
                                    .cast::<usize>();
                                let len5 = l4;
                                _rt::Vec::from_raw_parts(l3.cast(), len5, len5)
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l6 = i32::from(
                                    *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                );
                                ErrorCode::_lift(l6 as u8)
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result7
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn blockhash() -> Result<_rt::Vec<u8>, ErrorCode> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 3 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 3
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(
                        wasm_import_module = "catscope:witbot/transactionprocessor@0.1.0"
                    )]
                    unsafe extern "C" {
                        #[link_name = "blockhash"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result7 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0
                                    .add(::core::mem::size_of::<*const u8>())
                                    .cast::<*mut u8>();
                                let l4 = *ptr0
                                    .add(2 * ::core::mem::size_of::<*const u8>())
                                    .cast::<usize>();
                                let len5 = l4;
                                _rt::Vec::from_raw_parts(l3.cast(), len5, len5)
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l6 = i32::from(
                                    *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                );
                                ErrorCode::_lift(l6 as u8)
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result7
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn rent(size: u32) -> Result<u64, ErrorCode> {
                unsafe {
                    #[repr(align(8))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(
                        wasm_import_module = "catscope:witbot/transactionprocessor@0.1.0"
                    )]
                    unsafe extern "C" {
                        #[link_name = "rent"]
                        fn wit_import1(_: i32, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(_rt::as_i32(&size), ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result5 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0.add(8).cast::<i64>();
                                l3 as u64
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                ErrorCode::_lift(l4 as u8)
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result5
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod shooter {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type InputStream = super::super::super::wasi::io::streams::InputStream;
            pub type OutputStream = super::super::super::wasi::io::streams::OutputStream;
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum ErrorCode {
                Unknown,
                AccessDenied,
                NotSupported,
                InvalidArgument,
                OutOfMemory,
                Timeout,
            }
            impl ErrorCode {
                pub fn name(&self) -> &'static str {
                    match self {
                        ErrorCode::Unknown => "unknown",
                        ErrorCode::AccessDenied => "access-denied",
                        ErrorCode::NotSupported => "not-supported",
                        ErrorCode::InvalidArgument => "invalid-argument",
                        ErrorCode::OutOfMemory => "out-of-memory",
                        ErrorCode::Timeout => "timeout",
                    }
                }
                pub fn message(&self) -> &'static str {
                    match self {
                        ErrorCode::Unknown => "",
                        ErrorCode::AccessDenied => "",
                        ErrorCode::NotSupported => "",
                        ErrorCode::InvalidArgument => "",
                        ErrorCode::OutOfMemory => "",
                        ErrorCode::Timeout => "",
                    }
                }
            }
            impl ::core::fmt::Debug for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("ErrorCode")
                        .field("code", &(*self as i32))
                        .field("name", &self.name())
                        .field("message", &self.message())
                        .finish()
                }
            }
            impl ::core::fmt::Display for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    write!(f, "{} (error {})", self.name(), * self as i32)
                }
            }
            impl std::error::Error for ErrorCode {}
            impl ErrorCode {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> ErrorCode {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => ErrorCode::Unknown,
                        1 => ErrorCode::AccessDenied,
                        2 => ErrorCode::NotSupported,
                        3 => ErrorCode::InvalidArgument,
                        4 => ErrorCode::OutOfMemory,
                        5 => ErrorCode::Timeout,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct Subscription {
                handle: _rt::Resource<Subscription>,
            }
            impl Subscription {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for Subscription {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]subscription"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl Subscription {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn new() -> Self {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                        unsafe extern "C" {
                            #[link_name = "[constructor]subscription"]
                            fn wit_import0() -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0() -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0();
                        Subscription::from_handle(ret as u32)
                    }
                }
            }
            impl Subscription {
                #[allow(unused_unsafe, clippy::all)]
                /// Subscribe to a part of the graph.
                #[allow(async_fn_in_trait)]
                pub fn subscribe(
                    &self,
                    id: u64,
                    filter: u32,
                    depth: u32,
                ) -> wit_bindgen::rt::async_support::FutureReader<u32> {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                        unsafe extern "C" {
                            #[link_name = "[method]subscription.subscribe"]
                            fn wit_import0(_: i32, _: i64, _: i32, _: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(
                            _: i32,
                            _: i64,
                            _: i32,
                            _: i32,
                        ) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0(
                            (self).handle() as i32,
                            _rt::as_i64(&id),
                            _rt::as_i32(&filter),
                            _rt::as_i32(&depth),
                        );
                        wit_bindgen::rt::async_support::FutureReader::new(
                            ret as u32,
                            &super::super::super::wit_future::vtable1::VTABLE,
                        )
                    }
                }
            }
            impl Subscription {
                #[allow(unused_unsafe, clippy::all)]
                /// Cancel a subscription.
                #[allow(async_fn_in_trait)]
                pub fn cancel(&self, subid: u32) -> () {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                        unsafe extern "C" {
                            #[link_name = "[method]subscription.cancel"]
                            fn wit_import0(_: i32, _: i32);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32, _: i32) {
                            unreachable!()
                        }
                        wit_import0((self).handle() as i32, _rt::as_i32(&subid));
                    }
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn account_by_id(
                id: u64,
            ) -> wit_bindgen::rt::async_support::FutureReader<
                Result<_rt::Vec<u8>, ErrorCode>,
            > {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                    unsafe extern "C" {
                        #[link_name = "account-by-id"]
                        fn wit_import0(_: i64) -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0(_: i64) -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import0(_rt::as_i64(&id));
                    wit_bindgen::rt::async_support::FutureReader::new(
                        ret as u32,
                        &super::super::super::wit_future::vtable2::VTABLE,
                    )
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn account_by_pubkey(
                pubkey: &[u8],
            ) -> wit_bindgen::rt::async_support::FutureReader<
                Result<_rt::Vec<u8>, ErrorCode>,
            > {
                unsafe {
                    let vec0 = pubkey;
                    let ptr0 = vec0.as_ptr().cast::<u8>();
                    let len0 = vec0.len();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                    unsafe extern "C" {
                        #[link_name = "account-by-pubkey"]
                        fn wit_import1(_: *mut u8, _: usize) -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8, _: usize) -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import1(ptr0.cast_mut(), len0);
                    wit_bindgen::rt::async_support::FutureReader::new(
                        ret as u32,
                        &super::super::super::wit_future::vtable2::VTABLE,
                    )
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn subscribe(
                id: u64,
                filter: u32,
                depth: u32,
            ) -> Result<(Subscription, OutputStream), ErrorCode> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                    unsafe extern "C" {
                        #[link_name = "subscribe"]
                        fn wit_import1(_: i64, _: i32, _: i32, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(
                        _: i64,
                        _: i32,
                        _: i32,
                        _: *mut u8,
                    ) {
                        unreachable!()
                    }
                    wit_import1(
                        _rt::as_i64(&id),
                        _rt::as_i32(&filter),
                        _rt::as_i32(&depth),
                        ptr0,
                    );
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result6 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                let l4 = *ptr0.add(8).cast::<i32>();
                                (
                                    Subscription::from_handle(l3 as u32),
                                    super::super::super::wasi::io::streams::OutputStream::from_handle(
                                        l4 as u32,
                                    ),
                                )
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l5 = i32::from(*ptr0.add(4).cast::<u8>());
                                ErrorCode::_lift(l5 as u8)
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result6
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            /// close the stream to cancel the subscription.  The bloom filter is off of the pubkey, not the account_id.
            #[allow(async_fn_in_trait)]
            pub fn subscribebloomfilter(
                filter: u64,
            ) -> Result<(Subscription, OutputStream), ErrorCode> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                    unsafe extern "C" {
                        #[link_name = "subscribebloomfilter"]
                        fn wit_import1(_: i64, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i64, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(_rt::as_i64(&filter), ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result6 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                let l4 = *ptr0.add(8).cast::<i32>();
                                (
                                    Subscription::from_handle(l3 as u32),
                                    super::super::super::wasi::io::streams::OutputStream::from_handle(
                                        l4 as u32,
                                    ),
                                )
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l5 = i32::from(*ptr0.add(4).cast::<u8>());
                                ErrorCode::_lift(l5 as u8)
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result6
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn slot() -> Result<OutputStream, ErrorCode> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                    unsafe extern "C" {
                        #[link_name = "slot"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result5 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                super::super::super::wasi::io::streams::OutputStream::from_handle(
                                    l3 as u32,
                                )
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                ErrorCode::_lift(l4 as u8)
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result5
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn blockhash() -> Result<OutputStream, ErrorCode> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
                    unsafe extern "C" {
                        #[link_name = "blockhash"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result5 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                super::super::super::wasi::io::streams::OutputStream::from_handle(
                                    l3 as u32,
                                )
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                ErrorCode::_lift(l4 as u8)
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result5
                }
            }
        }
    }
}
#[rustfmt::skip]
#[allow(dead_code, clippy::all)]
pub mod wasi {
    pub mod cli {
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod environment {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            #[allow(unused_unsafe, clippy::all)]
            /// Get the POSIX-style environment variables.
            ///
            /// Each environment variable is provided as a pair of string variable names
            /// and string value.
            ///
            /// Morally, these are a value import, but until value imports are available
            /// in the component model, this import function should return the same
            /// values each time it is called.
            #[allow(async_fn_in_trait)]
            pub fn get_environment() -> _rt::Vec<(_rt::Vec<u8>, _rt::Vec<u8>)> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 2 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 2
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/environment@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-environment"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = *ptr0.add(0).cast::<*mut u8>();
                    let l3 = *ptr0
                        .add(::core::mem::size_of::<*const u8>())
                        .cast::<usize>();
                    let base10 = l2;
                    let len10 = l3;
                    let mut result10 = _rt::Vec::with_capacity(len10);
                    for i in 0..len10 {
                        let base = base10
                            .add(i * (4 * ::core::mem::size_of::<*const u8>()));
                        let e10 = {
                            let l4 = *base.add(0).cast::<*mut u8>();
                            let l5 = *base
                                .add(::core::mem::size_of::<*const u8>())
                                .cast::<usize>();
                            let len6 = l5;
                            let bytes6 = _rt::Vec::from_raw_parts(l4.cast(), len6, len6);
                            let l7 = *base
                                .add(2 * ::core::mem::size_of::<*const u8>())
                                .cast::<*mut u8>();
                            let l8 = *base
                                .add(3 * ::core::mem::size_of::<*const u8>())
                                .cast::<usize>();
                            let len9 = l8;
                            let bytes9 = _rt::Vec::from_raw_parts(l7.cast(), len9, len9);
                            (bytes6, bytes9)
                        };
                        result10.push(e10);
                    }
                    _rt::cabi_dealloc(
                        base10,
                        len10 * (4 * ::core::mem::size_of::<*const u8>()),
                        ::core::mem::size_of::<*const u8>(),
                    );
                    let result11 = result10;
                    result11
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            /// Get the POSIX-style arguments to the program.
            #[allow(async_fn_in_trait)]
            pub fn get_arguments() -> _rt::Vec<_rt::Vec<u8>> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 2 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 2
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/environment@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-arguments"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = *ptr0.add(0).cast::<*mut u8>();
                    let l3 = *ptr0
                        .add(::core::mem::size_of::<*const u8>())
                        .cast::<usize>();
                    let base7 = l2;
                    let len7 = l3;
                    let mut result7 = _rt::Vec::with_capacity(len7);
                    for i in 0..len7 {
                        let base = base7
                            .add(i * (2 * ::core::mem::size_of::<*const u8>()));
                        let e7 = {
                            let l4 = *base.add(0).cast::<*mut u8>();
                            let l5 = *base
                                .add(::core::mem::size_of::<*const u8>())
                                .cast::<usize>();
                            let len6 = l5;
                            let bytes6 = _rt::Vec::from_raw_parts(l4.cast(), len6, len6);
                            bytes6
                        };
                        result7.push(e7);
                    }
                    _rt::cabi_dealloc(
                        base7,
                        len7 * (2 * ::core::mem::size_of::<*const u8>()),
                        ::core::mem::size_of::<*const u8>(),
                    );
                    let result8 = result7;
                    result8
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            /// Return a path that programs should use as their initial current working
            /// directory, interpreting `.` as shorthand for this.
            #[allow(async_fn_in_trait)]
            pub fn initial_cwd() -> Option<_rt::Vec<u8>> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 3 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 3
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/environment@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "initial-cwd"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result6 = match l2 {
                        0 => None,
                        1 => {
                            let e = {
                                let l3 = *ptr0
                                    .add(::core::mem::size_of::<*const u8>())
                                    .cast::<*mut u8>();
                                let l4 = *ptr0
                                    .add(2 * ::core::mem::size_of::<*const u8>())
                                    .cast::<usize>();
                                let len5 = l4;
                                let bytes5 = _rt::Vec::from_raw_parts(
                                    l3.cast(),
                                    len5,
                                    len5,
                                );
                                bytes5
                            };
                            Some(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result6
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod exit {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            #[allow(unused_unsafe, clippy::all)]
            /// Exit the current instance and any linked instances.
            #[allow(async_fn_in_trait)]
            pub fn exit(status: Result<(), ()>) -> () {
                unsafe {
                    let result0 = match status {
                        Ok(_) => 0i32,
                        Err(_) => 1i32,
                    };
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/exit@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "exit"]
                        fn wit_import1(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i32) {
                        unreachable!()
                    }
                    wit_import1(result0);
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod stdin {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            pub type InputStream = super::super::super::wasi::io::streams::InputStream;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_stdin() -> InputStream {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/stdin@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-stdin"]
                        fn wit_import0() -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    super::super::super::wasi::io::streams::InputStream::from_handle(
                        ret as u32,
                    )
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod stdout {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            pub type OutputStream = super::super::super::wasi::io::streams::OutputStream;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_stdout() -> OutputStream {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/stdout@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-stdout"]
                        fn wit_import0() -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    super::super::super::wasi::io::streams::OutputStream::from_handle(
                        ret as u32,
                    )
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod stderr {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            pub type OutputStream = super::super::super::wasi::io::streams::OutputStream;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_stderr() -> OutputStream {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/stderr@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-stderr"]
                        fn wit_import0() -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    super::super::super::wasi::io::streams::OutputStream::from_handle(
                        ret as u32,
                    )
                }
            }
        }
        /// Terminal input.
        ///
        /// In the future, this may include functions for disabling echoing,
        /// disabling input buffering so that keyboard events are sent through
        /// immediately, querying supported features, and so on.
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod terminal_input {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            /// The input side of a terminal.
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct TerminalInput {
                handle: _rt::Resource<TerminalInput>,
            }
            impl TerminalInput {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for TerminalInput {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/terminal-input@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]terminal-input"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
        }
        /// Terminal output.
        ///
        /// In the future, this may include functions for querying the terminal
        /// size, being notified of terminal size changes, querying supported
        /// features, and so on.
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod terminal_output {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            /// The output side of a terminal.
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct TerminalOutput {
                handle: _rt::Resource<TerminalOutput>,
            }
            impl TerminalOutput {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for TerminalOutput {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/terminal-output@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]terminal-output"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
        }
        /// An interface providing an optional `terminal-input` for stdin as a
        /// link-time authority.
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod terminal_stdin {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type TerminalInput = super::super::super::wasi::cli::terminal_input::TerminalInput;
            #[allow(unused_unsafe, clippy::all)]
            /// If stdin is connected to a terminal, return a `terminal-input` handle
            /// allowing further interaction with it.
            #[allow(async_fn_in_trait)]
            pub fn get_terminal_stdin() -> Option<TerminalInput> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/terminal-stdin@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-terminal-stdin"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result4 = match l2 {
                        0 => None,
                        1 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                super::super::super::wasi::cli::terminal_input::TerminalInput::from_handle(
                                    l3 as u32,
                                )
                            };
                            Some(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result4
                }
            }
        }
        /// An interface providing an optional `terminal-output` for stdout as a
        /// link-time authority.
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod terminal_stdout {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type TerminalOutput = super::super::super::wasi::cli::terminal_output::TerminalOutput;
            #[allow(unused_unsafe, clippy::all)]
            /// If stdout is connected to a terminal, return a `terminal-output` handle
            /// allowing further interaction with it.
            #[allow(async_fn_in_trait)]
            pub fn get_terminal_stdout() -> Option<TerminalOutput> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/terminal-stdout@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-terminal-stdout"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result4 = match l2 {
                        0 => None,
                        1 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                super::super::super::wasi::cli::terminal_output::TerminalOutput::from_handle(
                                    l3 as u32,
                                )
                            };
                            Some(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result4
                }
            }
        }
        /// An interface providing an optional `terminal-output` for stderr as a
        /// link-time authority.
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod terminal_stderr {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type TerminalOutput = super::super::super::wasi::cli::terminal_output::TerminalOutput;
            #[allow(unused_unsafe, clippy::all)]
            /// If stderr is connected to a terminal, return a `terminal-output` handle
            /// allowing further interaction with it.
            #[allow(async_fn_in_trait)]
            pub fn get_terminal_stderr() -> Option<TerminalOutput> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:cli/terminal-stderr@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-terminal-stderr"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result4 = match l2 {
                        0 => None,
                        1 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                super::super::super::wasi::cli::terminal_output::TerminalOutput::from_handle(
                                    l3 as u32,
                                )
                            };
                            Some(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result4
                }
            }
        }
    }
    pub mod clocks {
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod monotonic_clock {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Pollable = super::super::super::wasi::io::poll::Pollable;
            pub type Instant = u64;
            pub type Duration = u64;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn now() -> Instant {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:clocks/monotonic-clock@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "now"]
                        fn wit_import0() -> i64;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i64 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    ret as u64
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn resolution() -> Duration {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:clocks/monotonic-clock@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "resolution"]
                        fn wit_import0() -> i64;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i64 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    ret as u64
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn subscribe_instant(when: Instant) -> Pollable {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:clocks/monotonic-clock@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "subscribe-instant"]
                        fn wit_import0(_: i64) -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0(_: i64) -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import0(_rt::as_i64(when));
                    super::super::super::wasi::io::poll::Pollable::from_handle(
                        ret as u32,
                    )
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn subscribe_duration(when: Duration) -> Pollable {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:clocks/monotonic-clock@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "subscribe-duration"]
                        fn wit_import0(_: i64) -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0(_: i64) -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import0(_rt::as_i64(when));
                    super::super::super::wasi::io::poll::Pollable::from_handle(
                        ret as u32,
                    )
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod wall_clock {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct Datetime {
                pub seconds: u64,
                pub nanoseconds: u32,
            }
            impl ::core::fmt::Debug for Datetime {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("Datetime")
                        .field("seconds", &self.seconds)
                        .field("nanoseconds", &self.nanoseconds)
                        .finish()
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn now() -> Datetime {
                unsafe {
                    #[repr(align(8))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:clocks/wall-clock@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "now"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = *ptr0.add(0).cast::<i64>();
                    let l3 = *ptr0.add(8).cast::<i32>();
                    let result4 = Datetime {
                        seconds: l2 as u64,
                        nanoseconds: l3 as u32,
                    };
                    result4
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn resolution() -> Datetime {
                unsafe {
                    #[repr(align(8))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:clocks/wall-clock@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "resolution"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = *ptr0.add(0).cast::<i64>();
                    let l3 = *ptr0.add(8).cast::<i32>();
                    let result4 = Datetime {
                        seconds: l2 as u64,
                        nanoseconds: l3 as u32,
                    };
                    result4
                }
            }
        }
    }
    pub mod filesystem {
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod types {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type InputStream = super::super::super::wasi::io::streams::InputStream;
            pub type OutputStream = super::super::super::wasi::io::streams::OutputStream;
            pub type Error = super::super::super::wasi::io::streams::Error;
            pub type Datetime = super::super::super::wasi::clocks::wall_clock::Datetime;
            pub type Filesize = u64;
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum DescriptorType {
                Unknown,
                BlockDevice,
                CharacterDevice,
                Directory,
                Fifo,
                SymbolicLink,
                RegularFile,
                Socket,
            }
            impl ::core::fmt::Debug for DescriptorType {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        DescriptorType::Unknown => {
                            f.debug_tuple("DescriptorType::Unknown").finish()
                        }
                        DescriptorType::BlockDevice => {
                            f.debug_tuple("DescriptorType::BlockDevice").finish()
                        }
                        DescriptorType::CharacterDevice => {
                            f.debug_tuple("DescriptorType::CharacterDevice").finish()
                        }
                        DescriptorType::Directory => {
                            f.debug_tuple("DescriptorType::Directory").finish()
                        }
                        DescriptorType::Fifo => {
                            f.debug_tuple("DescriptorType::Fifo").finish()
                        }
                        DescriptorType::SymbolicLink => {
                            f.debug_tuple("DescriptorType::SymbolicLink").finish()
                        }
                        DescriptorType::RegularFile => {
                            f.debug_tuple("DescriptorType::RegularFile").finish()
                        }
                        DescriptorType::Socket => {
                            f.debug_tuple("DescriptorType::Socket").finish()
                        }
                    }
                }
            }
            impl DescriptorType {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> DescriptorType {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => DescriptorType::Unknown,
                        1 => DescriptorType::BlockDevice,
                        2 => DescriptorType::CharacterDevice,
                        3 => DescriptorType::Directory,
                        4 => DescriptorType::Fifo,
                        5 => DescriptorType::SymbolicLink,
                        6 => DescriptorType::RegularFile,
                        7 => DescriptorType::Socket,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            wit_bindgen::rt::bitflags::bitflags! {
                #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)] pub
                struct DescriptorFlags : u8 { const READ = 1 << 0; const WRITE = 1 << 1;
                const FILE_INTEGRITY_SYNC = 1 << 2; const DATA_INTEGRITY_SYNC = 1 << 3;
                const REQUESTED_WRITE_SYNC = 1 << 4; const MUTATE_DIRECTORY = 1 << 5; }
            }
            wit_bindgen::rt::bitflags::bitflags! {
                #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)] pub
                struct PathFlags : u8 { const SYMLINK_FOLLOW = 1 << 0; }
            }
            wit_bindgen::rt::bitflags::bitflags! {
                #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)] pub
                struct OpenFlags : u8 { const CREATE = 1 << 0; const DIRECTORY = 1 << 1;
                const EXCLUSIVE = 1 << 2; const TRUNCATE = 1 << 3; }
            }
            pub type LinkCount = u64;
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct DescriptorStat {
                pub type_: DescriptorType,
                pub link_count: LinkCount,
                pub size: Filesize,
                pub data_access_timestamp: Option<Datetime>,
                pub data_modification_timestamp: Option<Datetime>,
                pub status_change_timestamp: Option<Datetime>,
            }
            impl ::core::fmt::Debug for DescriptorStat {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("DescriptorStat")
                        .field("type", &self.type_)
                        .field("link-count", &self.link_count)
                        .field("size", &self.size)
                        .field("data-access-timestamp", &self.data_access_timestamp)
                        .field(
                            "data-modification-timestamp",
                            &self.data_modification_timestamp,
                        )
                        .field("status-change-timestamp", &self.status_change_timestamp)
                        .finish()
                }
            }
            #[derive(Clone, Copy)]
            pub enum NewTimestamp {
                NoChange,
                Now,
                Timestamp(Datetime),
            }
            impl ::core::fmt::Debug for NewTimestamp {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        NewTimestamp::NoChange => {
                            f.debug_tuple("NewTimestamp::NoChange").finish()
                        }
                        NewTimestamp::Now => f.debug_tuple("NewTimestamp::Now").finish(),
                        NewTimestamp::Timestamp(e) => {
                            f.debug_tuple("NewTimestamp::Timestamp").field(e).finish()
                        }
                    }
                }
            }
            #[derive(Clone)]
            pub struct DirectoryEntry {
                pub type_: DescriptorType,
                pub name: _rt::Vec<u8>,
            }
            impl ::core::fmt::Debug for DirectoryEntry {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("DirectoryEntry")
                        .field("type", &self.type_)
                        .field("name", &self.name)
                        .finish()
                }
            }
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum ErrorCode {
                Access,
                WouldBlock,
                Already,
                BadDescriptor,
                Busy,
                Deadlock,
                Quota,
                Exist,
                FileTooLarge,
                IllegalByteSequence,
                InProgress,
                Interrupted,
                Invalid,
                Io,
                IsDirectory,
                Loop,
                TooManyLinks,
                MessageSize,
                NameTooLong,
                NoDevice,
                NoEntry,
                NoLock,
                InsufficientMemory,
                InsufficientSpace,
                NotDirectory,
                NotEmpty,
                NotRecoverable,
                Unsupported,
                NoTty,
                NoSuchDevice,
                Overflow,
                NotPermitted,
                Pipe,
                ReadOnly,
                InvalidSeek,
                TextFileBusy,
                CrossDevice,
            }
            impl ErrorCode {
                pub fn name(&self) -> &'static str {
                    match self {
                        ErrorCode::Access => "access",
                        ErrorCode::WouldBlock => "would-block",
                        ErrorCode::Already => "already",
                        ErrorCode::BadDescriptor => "bad-descriptor",
                        ErrorCode::Busy => "busy",
                        ErrorCode::Deadlock => "deadlock",
                        ErrorCode::Quota => "quota",
                        ErrorCode::Exist => "exist",
                        ErrorCode::FileTooLarge => "file-too-large",
                        ErrorCode::IllegalByteSequence => "illegal-byte-sequence",
                        ErrorCode::InProgress => "in-progress",
                        ErrorCode::Interrupted => "interrupted",
                        ErrorCode::Invalid => "invalid",
                        ErrorCode::Io => "io",
                        ErrorCode::IsDirectory => "is-directory",
                        ErrorCode::Loop => "loop",
                        ErrorCode::TooManyLinks => "too-many-links",
                        ErrorCode::MessageSize => "message-size",
                        ErrorCode::NameTooLong => "name-too-long",
                        ErrorCode::NoDevice => "no-device",
                        ErrorCode::NoEntry => "no-entry",
                        ErrorCode::NoLock => "no-lock",
                        ErrorCode::InsufficientMemory => "insufficient-memory",
                        ErrorCode::InsufficientSpace => "insufficient-space",
                        ErrorCode::NotDirectory => "not-directory",
                        ErrorCode::NotEmpty => "not-empty",
                        ErrorCode::NotRecoverable => "not-recoverable",
                        ErrorCode::Unsupported => "unsupported",
                        ErrorCode::NoTty => "no-tty",
                        ErrorCode::NoSuchDevice => "no-such-device",
                        ErrorCode::Overflow => "overflow",
                        ErrorCode::NotPermitted => "not-permitted",
                        ErrorCode::Pipe => "pipe",
                        ErrorCode::ReadOnly => "read-only",
                        ErrorCode::InvalidSeek => "invalid-seek",
                        ErrorCode::TextFileBusy => "text-file-busy",
                        ErrorCode::CrossDevice => "cross-device",
                    }
                }
                pub fn message(&self) -> &'static str {
                    match self {
                        ErrorCode::Access => "",
                        ErrorCode::WouldBlock => "",
                        ErrorCode::Already => "",
                        ErrorCode::BadDescriptor => "",
                        ErrorCode::Busy => "",
                        ErrorCode::Deadlock => "",
                        ErrorCode::Quota => "",
                        ErrorCode::Exist => "",
                        ErrorCode::FileTooLarge => "",
                        ErrorCode::IllegalByteSequence => "",
                        ErrorCode::InProgress => "",
                        ErrorCode::Interrupted => "",
                        ErrorCode::Invalid => "",
                        ErrorCode::Io => "",
                        ErrorCode::IsDirectory => "",
                        ErrorCode::Loop => "",
                        ErrorCode::TooManyLinks => "",
                        ErrorCode::MessageSize => "",
                        ErrorCode::NameTooLong => "",
                        ErrorCode::NoDevice => "",
                        ErrorCode::NoEntry => "",
                        ErrorCode::NoLock => "",
                        ErrorCode::InsufficientMemory => "",
                        ErrorCode::InsufficientSpace => "",
                        ErrorCode::NotDirectory => "",
                        ErrorCode::NotEmpty => "",
                        ErrorCode::NotRecoverable => "",
                        ErrorCode::Unsupported => "",
                        ErrorCode::NoTty => "",
                        ErrorCode::NoSuchDevice => "",
                        ErrorCode::Overflow => "",
                        ErrorCode::NotPermitted => "",
                        ErrorCode::Pipe => "",
                        ErrorCode::ReadOnly => "",
                        ErrorCode::InvalidSeek => "",
                        ErrorCode::TextFileBusy => "",
                        ErrorCode::CrossDevice => "",
                    }
                }
            }
            impl ::core::fmt::Debug for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("ErrorCode")
                        .field("code", &(*self as i32))
                        .field("name", &self.name())
                        .field("message", &self.message())
                        .finish()
                }
            }
            impl ::core::fmt::Display for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    write!(f, "{} (error {})", self.name(), * self as i32)
                }
            }
            impl std::error::Error for ErrorCode {}
            impl ErrorCode {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> ErrorCode {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => ErrorCode::Access,
                        1 => ErrorCode::WouldBlock,
                        2 => ErrorCode::Already,
                        3 => ErrorCode::BadDescriptor,
                        4 => ErrorCode::Busy,
                        5 => ErrorCode::Deadlock,
                        6 => ErrorCode::Quota,
                        7 => ErrorCode::Exist,
                        8 => ErrorCode::FileTooLarge,
                        9 => ErrorCode::IllegalByteSequence,
                        10 => ErrorCode::InProgress,
                        11 => ErrorCode::Interrupted,
                        12 => ErrorCode::Invalid,
                        13 => ErrorCode::Io,
                        14 => ErrorCode::IsDirectory,
                        15 => ErrorCode::Loop,
                        16 => ErrorCode::TooManyLinks,
                        17 => ErrorCode::MessageSize,
                        18 => ErrorCode::NameTooLong,
                        19 => ErrorCode::NoDevice,
                        20 => ErrorCode::NoEntry,
                        21 => ErrorCode::NoLock,
                        22 => ErrorCode::InsufficientMemory,
                        23 => ErrorCode::InsufficientSpace,
                        24 => ErrorCode::NotDirectory,
                        25 => ErrorCode::NotEmpty,
                        26 => ErrorCode::NotRecoverable,
                        27 => ErrorCode::Unsupported,
                        28 => ErrorCode::NoTty,
                        29 => ErrorCode::NoSuchDevice,
                        30 => ErrorCode::Overflow,
                        31 => ErrorCode::NotPermitted,
                        32 => ErrorCode::Pipe,
                        33 => ErrorCode::ReadOnly,
                        34 => ErrorCode::InvalidSeek,
                        35 => ErrorCode::TextFileBusy,
                        36 => ErrorCode::CrossDevice,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum Advice {
                Normal,
                Sequential,
                Random,
                WillNeed,
                DontNeed,
                NoReuse,
            }
            impl ::core::fmt::Debug for Advice {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        Advice::Normal => f.debug_tuple("Advice::Normal").finish(),
                        Advice::Sequential => {
                            f.debug_tuple("Advice::Sequential").finish()
                        }
                        Advice::Random => f.debug_tuple("Advice::Random").finish(),
                        Advice::WillNeed => f.debug_tuple("Advice::WillNeed").finish(),
                        Advice::DontNeed => f.debug_tuple("Advice::DontNeed").finish(),
                        Advice::NoReuse => f.debug_tuple("Advice::NoReuse").finish(),
                    }
                }
            }
            impl Advice {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> Advice {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => Advice::Normal,
                        1 => Advice::Sequential,
                        2 => Advice::Random,
                        3 => Advice::WillNeed,
                        4 => Advice::DontNeed,
                        5 => Advice::NoReuse,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct MetadataHashValue {
                pub lower: u64,
                pub upper: u64,
            }
            impl ::core::fmt::Debug for MetadataHashValue {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("MetadataHashValue")
                        .field("lower", &self.lower)
                        .field("upper", &self.upper)
                        .finish()
                }
            }
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct Descriptor {
                handle: _rt::Resource<Descriptor>,
            }
            impl Descriptor {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for Descriptor {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]descriptor"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct DirectoryEntryStream {
                handle: _rt::Resource<DirectoryEntryStream>,
            }
            impl DirectoryEntryStream {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for DirectoryEntryStream {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]directory-entry-stream"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn read_via_stream(
                    &self,
                    offset: Filesize,
                ) -> Result<InputStream, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 8],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.read-via-stream"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(offset), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(4).cast::<i32>();
                                    super::super::super::wasi::io::streams::InputStream::from_handle(
                                        l3 as u32,
                                    )
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn write_via_stream(
                    &self,
                    offset: Filesize,
                ) -> Result<OutputStream, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 8],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.write-via-stream"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(offset), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(4).cast::<i32>();
                                    super::super::super::wasi::io::streams::OutputStream::from_handle(
                                        l3 as u32,
                                    )
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn append_via_stream(&self) -> Result<OutputStream, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 8],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.append-via-stream"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(4).cast::<i32>();
                                    super::super::super::wasi::io::streams::OutputStream::from_handle(
                                        l3 as u32,
                                    )
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn advise(
                    &self,
                    offset: Filesize,
                    length: Filesize,
                    advice: Advice,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.advise"]
                            fn wit_import1(_: i32, _: i64, _: i64, _: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(
                            _: i32,
                            _: i64,
                            _: i64,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import1(
                            (self).handle() as i32,
                            _rt::as_i64(offset),
                            _rt::as_i64(length),
                            advice.clone() as i32,
                            ptr0,
                        );
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    ErrorCode::_lift(l3 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn sync_data(&self) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.sync-data"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    ErrorCode::_lift(l3 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn get_flags(&self) -> Result<DescriptorFlags, ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.get-flags"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    DescriptorFlags::empty()
                                        | DescriptorFlags::from_bits_retain(((l3 as u8) << 0) as _)
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(1).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn get_type(&self) -> Result<DescriptorType, ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.get-type"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    DescriptorType::_lift(l3 as u8)
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(1).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_size(&self, size: Filesize) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.set-size"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(size), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    ErrorCode::_lift(l3 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_times(
                    &self,
                    data_access_timestamp: NewTimestamp,
                    data_modification_timestamp: NewTimestamp,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let (result1_0, result1_1, result1_2) = match data_access_timestamp {
                            NewTimestamp::NoChange => (0i32, 0i64, 0i32),
                            NewTimestamp::Now => (1i32, 0i64, 0i32),
                            NewTimestamp::Timestamp(e) => {
                                let super::super::super::wasi::clocks::wall_clock::Datetime {
                                    seconds: seconds0,
                                    nanoseconds: nanoseconds0,
                                } = e;
                                (2i32, _rt::as_i64(seconds0), _rt::as_i32(nanoseconds0))
                            }
                        };
                        let (result3_0, result3_1, result3_2) = match data_modification_timestamp {
                            NewTimestamp::NoChange => (0i32, 0i64, 0i32),
                            NewTimestamp::Now => (1i32, 0i64, 0i32),
                            NewTimestamp::Timestamp(e) => {
                                let super::super::super::wasi::clocks::wall_clock::Datetime {
                                    seconds: seconds2,
                                    nanoseconds: nanoseconds2,
                                } = e;
                                (2i32, _rt::as_i64(seconds2), _rt::as_i32(nanoseconds2))
                            }
                        };
                        let ptr4 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.set-times"]
                            fn wit_import5(
                                _: i32,
                                _: i32,
                                _: i64,
                                _: i32,
                                _: i32,
                                _: i64,
                                _: i32,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import5(
                            _: i32,
                            _: i32,
                            _: i64,
                            _: i32,
                            _: i32,
                            _: i64,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import5(
                            (self).handle() as i32,
                            result1_0,
                            result1_1,
                            result1_2,
                            result3_0,
                            result3_1,
                            result3_2,
                            ptr4,
                        );
                        let l6 = i32::from(*ptr4.add(0).cast::<u8>());
                        let result8 = match l6 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l7 = i32::from(*ptr4.add(1).cast::<u8>());
                                    ErrorCode::_lift(l7 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result8
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn read(
                    &self,
                    length: Filesize,
                    offset: Filesize,
                ) -> Result<(_rt::Vec<u8>, bool), ErrorCode> {
                    unsafe {
                        #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                        #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                        struct RetArea(
                            [::core::mem::MaybeUninit<
                                u8,
                            >; 4 * ::core::mem::size_of::<*const u8>()],
                        );
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 4
                                * ::core::mem::size_of::<*const u8>()],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.read"]
                            fn wit_import1(_: i32, _: i64, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(
                            _: i32,
                            _: i64,
                            _: i64,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import1(
                            (self).handle() as i32,
                            _rt::as_i64(length),
                            _rt::as_i64(offset),
                            ptr0,
                        );
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result8 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0
                                        .add(::core::mem::size_of::<*const u8>())
                                        .cast::<*mut u8>();
                                    let l4 = *ptr0
                                        .add(2 * ::core::mem::size_of::<*const u8>())
                                        .cast::<usize>();
                                    let len5 = l4;
                                    let l6 = i32::from(
                                        *ptr0
                                            .add(3 * ::core::mem::size_of::<*const u8>())
                                            .cast::<u8>(),
                                    );
                                    (
                                        _rt::Vec::from_raw_parts(l3.cast(), len5, len5),
                                        _rt::bool_lift(l6 as u8),
                                    )
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l7 = i32::from(
                                        *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                    );
                                    ErrorCode::_lift(l7 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result8
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn write(
                    &self,
                    buffer: &[u8],
                    offset: Filesize,
                ) -> Result<Filesize, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let vec0 = buffer;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.write"]
                            fn wit_import2(
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: i64,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import2(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: i64,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import2(
                            (self).handle() as i32,
                            ptr0.cast_mut(),
                            len0,
                            _rt::as_i64(offset),
                            ptr1,
                        );
                        let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                        let result6 = match l3 {
                            0 => {
                                let e = {
                                    let l4 = *ptr1.add(8).cast::<i64>();
                                    l4 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l5 = i32::from(*ptr1.add(8).cast::<u8>());
                                    ErrorCode::_lift(l5 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn read_directory(&self) -> Result<DirectoryEntryStream, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 8],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.read-directory"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(4).cast::<i32>();
                                    DirectoryEntryStream::from_handle(l3 as u32)
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn sync(&self) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.sync"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    ErrorCode::_lift(l3 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn create_directory_at(&self, path: &[u8]) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let vec0 = path;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.create-directory-at"]
                            fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import2(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import2((self).handle() as i32, ptr0.cast_mut(), len0, ptr1);
                        let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                        let result5 = match l3 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr1.add(1).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn stat(&self) -> Result<DescriptorStat, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 104]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 104],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.stat"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result16 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(8).cast::<u8>());
                                    let l4 = *ptr0.add(16).cast::<i64>();
                                    let l5 = *ptr0.add(24).cast::<i64>();
                                    let l6 = i32::from(*ptr0.add(32).cast::<u8>());
                                    let l9 = i32::from(*ptr0.add(56).cast::<u8>());
                                    let l12 = i32::from(*ptr0.add(80).cast::<u8>());
                                    DescriptorStat {
                                        type_: DescriptorType::_lift(l3 as u8),
                                        link_count: l4 as u64,
                                        size: l5 as u64,
                                        data_access_timestamp: match l6 {
                                            0 => None,
                                            1 => {
                                                let e = {
                                                    let l7 = *ptr0.add(40).cast::<i64>();
                                                    let l8 = *ptr0.add(48).cast::<i32>();
                                                    super::super::super::wasi::clocks::wall_clock::Datetime {
                                                        seconds: l7 as u64,
                                                        nanoseconds: l8 as u32,
                                                    }
                                                };
                                                Some(e)
                                            }
                                            _ => _rt::invalid_enum_discriminant(),
                                        },
                                        data_modification_timestamp: match l9 {
                                            0 => None,
                                            1 => {
                                                let e = {
                                                    let l10 = *ptr0.add(64).cast::<i64>();
                                                    let l11 = *ptr0.add(72).cast::<i32>();
                                                    super::super::super::wasi::clocks::wall_clock::Datetime {
                                                        seconds: l10 as u64,
                                                        nanoseconds: l11 as u32,
                                                    }
                                                };
                                                Some(e)
                                            }
                                            _ => _rt::invalid_enum_discriminant(),
                                        },
                                        status_change_timestamp: match l12 {
                                            0 => None,
                                            1 => {
                                                let e = {
                                                    let l13 = *ptr0.add(88).cast::<i64>();
                                                    let l14 = *ptr0.add(96).cast::<i32>();
                                                    super::super::super::wasi::clocks::wall_clock::Datetime {
                                                        seconds: l13 as u64,
                                                        nanoseconds: l14 as u32,
                                                    }
                                                };
                                                Some(e)
                                            }
                                            _ => _rt::invalid_enum_discriminant(),
                                        },
                                    }
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l15 = i32::from(*ptr0.add(8).cast::<u8>());
                                    ErrorCode::_lift(l15 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result16
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn stat_at(
                    &self,
                    path_flags: PathFlags,
                    path: &[u8],
                ) -> Result<DescriptorStat, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 104]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 104],
                        );
                        let flags0 = path_flags;
                        let vec1 = path;
                        let ptr1 = vec1.as_ptr().cast::<u8>();
                        let len1 = vec1.len();
                        let ptr2 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.stat-at"]
                            fn wit_import3(
                                _: i32,
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import3(
                            _: i32,
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import3(
                            (self).handle() as i32,
                            (flags0.bits() >> 0) as i32,
                            ptr1.cast_mut(),
                            len1,
                            ptr2,
                        );
                        let l4 = i32::from(*ptr2.add(0).cast::<u8>());
                        let result18 = match l4 {
                            0 => {
                                let e = {
                                    let l5 = i32::from(*ptr2.add(8).cast::<u8>());
                                    let l6 = *ptr2.add(16).cast::<i64>();
                                    let l7 = *ptr2.add(24).cast::<i64>();
                                    let l8 = i32::from(*ptr2.add(32).cast::<u8>());
                                    let l11 = i32::from(*ptr2.add(56).cast::<u8>());
                                    let l14 = i32::from(*ptr2.add(80).cast::<u8>());
                                    DescriptorStat {
                                        type_: DescriptorType::_lift(l5 as u8),
                                        link_count: l6 as u64,
                                        size: l7 as u64,
                                        data_access_timestamp: match l8 {
                                            0 => None,
                                            1 => {
                                                let e = {
                                                    let l9 = *ptr2.add(40).cast::<i64>();
                                                    let l10 = *ptr2.add(48).cast::<i32>();
                                                    super::super::super::wasi::clocks::wall_clock::Datetime {
                                                        seconds: l9 as u64,
                                                        nanoseconds: l10 as u32,
                                                    }
                                                };
                                                Some(e)
                                            }
                                            _ => _rt::invalid_enum_discriminant(),
                                        },
                                        data_modification_timestamp: match l11 {
                                            0 => None,
                                            1 => {
                                                let e = {
                                                    let l12 = *ptr2.add(64).cast::<i64>();
                                                    let l13 = *ptr2.add(72).cast::<i32>();
                                                    super::super::super::wasi::clocks::wall_clock::Datetime {
                                                        seconds: l12 as u64,
                                                        nanoseconds: l13 as u32,
                                                    }
                                                };
                                                Some(e)
                                            }
                                            _ => _rt::invalid_enum_discriminant(),
                                        },
                                        status_change_timestamp: match l14 {
                                            0 => None,
                                            1 => {
                                                let e = {
                                                    let l15 = *ptr2.add(88).cast::<i64>();
                                                    let l16 = *ptr2.add(96).cast::<i32>();
                                                    super::super::super::wasi::clocks::wall_clock::Datetime {
                                                        seconds: l15 as u64,
                                                        nanoseconds: l16 as u32,
                                                    }
                                                };
                                                Some(e)
                                            }
                                            _ => _rt::invalid_enum_discriminant(),
                                        },
                                    }
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l17 = i32::from(*ptr2.add(8).cast::<u8>());
                                    ErrorCode::_lift(l17 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result18
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_times_at(
                    &self,
                    path_flags: PathFlags,
                    path: &[u8],
                    data_access_timestamp: NewTimestamp,
                    data_modification_timestamp: NewTimestamp,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let flags0 = path_flags;
                        let vec1 = path;
                        let ptr1 = vec1.as_ptr().cast::<u8>();
                        let len1 = vec1.len();
                        let (result3_0, result3_1, result3_2) = match data_access_timestamp {
                            NewTimestamp::NoChange => (0i32, 0i64, 0i32),
                            NewTimestamp::Now => (1i32, 0i64, 0i32),
                            NewTimestamp::Timestamp(e) => {
                                let super::super::super::wasi::clocks::wall_clock::Datetime {
                                    seconds: seconds2,
                                    nanoseconds: nanoseconds2,
                                } = e;
                                (2i32, _rt::as_i64(seconds2), _rt::as_i32(nanoseconds2))
                            }
                        };
                        let (result5_0, result5_1, result5_2) = match data_modification_timestamp {
                            NewTimestamp::NoChange => (0i32, 0i64, 0i32),
                            NewTimestamp::Now => (1i32, 0i64, 0i32),
                            NewTimestamp::Timestamp(e) => {
                                let super::super::super::wasi::clocks::wall_clock::Datetime {
                                    seconds: seconds4,
                                    nanoseconds: nanoseconds4,
                                } = e;
                                (2i32, _rt::as_i64(seconds4), _rt::as_i32(nanoseconds4))
                            }
                        };
                        let ptr6 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.set-times-at"]
                            fn wit_import7(
                                _: i32,
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: i32,
                                _: i64,
                                _: i32,
                                _: i32,
                                _: i64,
                                _: i32,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import7(
                            _: i32,
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: i32,
                            _: i64,
                            _: i32,
                            _: i32,
                            _: i64,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import7(
                            (self).handle() as i32,
                            (flags0.bits() >> 0) as i32,
                            ptr1.cast_mut(),
                            len1,
                            result3_0,
                            result3_1,
                            result3_2,
                            result5_0,
                            result5_1,
                            result5_2,
                            ptr6,
                        );
                        let l8 = i32::from(*ptr6.add(0).cast::<u8>());
                        let result10 = match l8 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l9 = i32::from(*ptr6.add(1).cast::<u8>());
                                    ErrorCode::_lift(l9 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result10
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn link_at(
                    &self,
                    old_path_flags: PathFlags,
                    old_path: &[u8],
                    new_descriptor: &Descriptor,
                    new_path: &[u8],
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let flags0 = old_path_flags;
                        let vec1 = old_path;
                        let ptr1 = vec1.as_ptr().cast::<u8>();
                        let len1 = vec1.len();
                        let vec2 = new_path;
                        let ptr2 = vec2.as_ptr().cast::<u8>();
                        let len2 = vec2.len();
                        let ptr3 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.link-at"]
                            fn wit_import4(
                                _: i32,
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import4(
                            _: i32,
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import4(
                            (self).handle() as i32,
                            (flags0.bits() >> 0) as i32,
                            ptr1.cast_mut(),
                            len1,
                            (new_descriptor).handle() as i32,
                            ptr2.cast_mut(),
                            len2,
                            ptr3,
                        );
                        let l5 = i32::from(*ptr3.add(0).cast::<u8>());
                        let result7 = match l5 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l6 = i32::from(*ptr3.add(1).cast::<u8>());
                                    ErrorCode::_lift(l6 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn open_at(
                    &self,
                    path_flags: PathFlags,
                    path: &[u8],
                    open_flags: OpenFlags,
                    flags: DescriptorFlags,
                ) -> Result<Descriptor, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 8],
                        );
                        let flags0 = path_flags;
                        let vec1 = path;
                        let ptr1 = vec1.as_ptr().cast::<u8>();
                        let len1 = vec1.len();
                        let flags2 = open_flags;
                        let flags3 = flags;
                        let ptr4 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.open-at"]
                            fn wit_import5(
                                _: i32,
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: i32,
                                _: i32,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import5(
                            _: i32,
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: i32,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import5(
                            (self).handle() as i32,
                            (flags0.bits() >> 0) as i32,
                            ptr1.cast_mut(),
                            len1,
                            (flags2.bits() >> 0) as i32,
                            (flags3.bits() >> 0) as i32,
                            ptr4,
                        );
                        let l6 = i32::from(*ptr4.add(0).cast::<u8>());
                        let result9 = match l6 {
                            0 => {
                                let e = {
                                    let l7 = *ptr4.add(4).cast::<i32>();
                                    Descriptor::from_handle(l7 as u32)
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l8 = i32::from(*ptr4.add(4).cast::<u8>());
                                    ErrorCode::_lift(l8 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result9
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn readlink_at(
                    &self,
                    path: &[u8],
                ) -> Result<_rt::Vec<u8>, ErrorCode> {
                    unsafe {
                        #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                        #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                        struct RetArea(
                            [::core::mem::MaybeUninit<
                                u8,
                            >; 3 * ::core::mem::size_of::<*const u8>()],
                        );
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 3
                                * ::core::mem::size_of::<*const u8>()],
                        );
                        let vec0 = path;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.readlink-at"]
                            fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import2(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import2((self).handle() as i32, ptr0.cast_mut(), len0, ptr1);
                        let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                        let result8 = match l3 {
                            0 => {
                                let e = {
                                    let l4 = *ptr1
                                        .add(::core::mem::size_of::<*const u8>())
                                        .cast::<*mut u8>();
                                    let l5 = *ptr1
                                        .add(2 * ::core::mem::size_of::<*const u8>())
                                        .cast::<usize>();
                                    let len6 = l5;
                                    let bytes6 = _rt::Vec::from_raw_parts(
                                        l4.cast(),
                                        len6,
                                        len6,
                                    );
                                    bytes6
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l7 = i32::from(
                                        *ptr1.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                    );
                                    ErrorCode::_lift(l7 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result8
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn remove_directory_at(&self, path: &[u8]) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let vec0 = path;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.remove-directory-at"]
                            fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import2(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import2((self).handle() as i32, ptr0.cast_mut(), len0, ptr1);
                        let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                        let result5 = match l3 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr1.add(1).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn rename_at(
                    &self,
                    old_path: &[u8],
                    new_descriptor: &Descriptor,
                    new_path: &[u8],
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let vec0 = old_path;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let vec1 = new_path;
                        let ptr1 = vec1.as_ptr().cast::<u8>();
                        let len1 = vec1.len();
                        let ptr2 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.rename-at"]
                            fn wit_import3(
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import3(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import3(
                            (self).handle() as i32,
                            ptr0.cast_mut(),
                            len0,
                            (new_descriptor).handle() as i32,
                            ptr1.cast_mut(),
                            len1,
                            ptr2,
                        );
                        let l4 = i32::from(*ptr2.add(0).cast::<u8>());
                        let result6 = match l4 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l5 = i32::from(*ptr2.add(1).cast::<u8>());
                                    ErrorCode::_lift(l5 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn symlink_at(
                    &self,
                    old_path: &[u8],
                    new_path: &[u8],
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let vec0 = old_path;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let vec1 = new_path;
                        let ptr1 = vec1.as_ptr().cast::<u8>();
                        let len1 = vec1.len();
                        let ptr2 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.symlink-at"]
                            fn wit_import3(
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: *mut u8,
                                _: usize,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import3(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import3(
                            (self).handle() as i32,
                            ptr0.cast_mut(),
                            len0,
                            ptr1.cast_mut(),
                            len1,
                            ptr2,
                        );
                        let l4 = i32::from(*ptr2.add(0).cast::<u8>());
                        let result6 = match l4 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l5 = i32::from(*ptr2.add(1).cast::<u8>());
                                    ErrorCode::_lift(l5 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn unlink_file_at(&self, path: &[u8]) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let vec0 = path;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.unlink-file-at"]
                            fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import2(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import2((self).handle() as i32, ptr0.cast_mut(), len0, ptr1);
                        let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                        let result5 = match l3 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr1.add(1).cast::<u8>());
                                    ErrorCode::_lift(l4 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn is_same_object(&self, other: &Descriptor) -> bool {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.is-same-object"]
                            fn wit_import0(_: i32, _: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32, _: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0(
                            (self).handle() as i32,
                            (other).handle() as i32,
                        );
                        _rt::bool_lift(ret as u8)
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn metadata_hash(&self) -> Result<MetadataHashValue, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 24]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 24],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.metadata-hash"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result6 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    let l4 = *ptr0.add(16).cast::<i64>();
                                    MetadataHashValue {
                                        lower: l3 as u64,
                                        upper: l4 as u64,
                                    }
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l5 = i32::from(*ptr0.add(8).cast::<u8>());
                                    ErrorCode::_lift(l5 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl Descriptor {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn metadata_hash_at(
                    &self,
                    path_flags: PathFlags,
                    path: &[u8],
                ) -> Result<MetadataHashValue, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 24]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 24],
                        );
                        let flags0 = path_flags;
                        let vec1 = path;
                        let ptr1 = vec1.as_ptr().cast::<u8>();
                        let len1 = vec1.len();
                        let ptr2 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]descriptor.metadata-hash-at"]
                            fn wit_import3(
                                _: i32,
                                _: i32,
                                _: *mut u8,
                                _: usize,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import3(
                            _: i32,
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import3(
                            (self).handle() as i32,
                            (flags0.bits() >> 0) as i32,
                            ptr1.cast_mut(),
                            len1,
                            ptr2,
                        );
                        let l4 = i32::from(*ptr2.add(0).cast::<u8>());
                        let result8 = match l4 {
                            0 => {
                                let e = {
                                    let l5 = *ptr2.add(8).cast::<i64>();
                                    let l6 = *ptr2.add(16).cast::<i64>();
                                    MetadataHashValue {
                                        lower: l5 as u64,
                                        upper: l6 as u64,
                                    }
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l7 = i32::from(*ptr2.add(8).cast::<u8>());
                                    ErrorCode::_lift(l7 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result8
                    }
                }
            }
            impl DirectoryEntryStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn read_directory_entry(
                    &self,
                ) -> Result<Option<DirectoryEntry>, ErrorCode> {
                    unsafe {
                        #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                        #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                        struct RetArea(
                            [::core::mem::MaybeUninit<
                                u8,
                            >; 5 * ::core::mem::size_of::<*const u8>()],
                        );
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 5
                                * ::core::mem::size_of::<*const u8>()],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]directory-entry-stream.read-directory-entry"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result9 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(
                                        *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                    );
                                    match l3 {
                                        0 => None,
                                        1 => {
                                            let e = {
                                                let l4 = i32::from(
                                                    *ptr0
                                                        .add(2 * ::core::mem::size_of::<*const u8>())
                                                        .cast::<u8>(),
                                                );
                                                let l5 = *ptr0
                                                    .add(3 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<*mut u8>();
                                                let l6 = *ptr0
                                                    .add(4 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<usize>();
                                                let len7 = l6;
                                                let bytes7 = _rt::Vec::from_raw_parts(
                                                    l5.cast(),
                                                    len7,
                                                    len7,
                                                );
                                                DirectoryEntry {
                                                    type_: DescriptorType::_lift(l4 as u8),
                                                    name: bytes7,
                                                }
                                            };
                                            Some(e)
                                        }
                                        _ => _rt::invalid_enum_discriminant(),
                                    }
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l8 = i32::from(
                                        *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                    );
                                    ErrorCode::_lift(l8 as u8)
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result9
                    }
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn filesystem_error_code(err: &Error) -> Option<ErrorCode> {
                unsafe {
                    #[repr(align(1))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 2]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:filesystem/types@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "filesystem-error-code"]
                        fn wit_import1(_: i32, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1((err).handle() as i32, ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result4 = match l2 {
                        0 => None,
                        1 => {
                            let e = {
                                let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                ErrorCode::_lift(l3 as u8)
                            };
                            Some(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result4
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod preopens {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Descriptor = super::super::super::wasi::filesystem::types::Descriptor;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_directories() -> _rt::Vec<(Descriptor, _rt::Vec<u8>)> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 2 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 2
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:filesystem/preopens@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-directories"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = *ptr0.add(0).cast::<*mut u8>();
                    let l3 = *ptr0
                        .add(::core::mem::size_of::<*const u8>())
                        .cast::<usize>();
                    let base8 = l2;
                    let len8 = l3;
                    let mut result8 = _rt::Vec::with_capacity(len8);
                    for i in 0..len8 {
                        let base = base8
                            .add(i * (3 * ::core::mem::size_of::<*const u8>()));
                        let e8 = {
                            let l4 = *base.add(0).cast::<i32>();
                            let l5 = *base
                                .add(::core::mem::size_of::<*const u8>())
                                .cast::<*mut u8>();
                            let l6 = *base
                                .add(2 * ::core::mem::size_of::<*const u8>())
                                .cast::<usize>();
                            let len7 = l6;
                            let bytes7 = _rt::Vec::from_raw_parts(l5.cast(), len7, len7);
                            (
                                super::super::super::wasi::filesystem::types::Descriptor::from_handle(
                                    l4 as u32,
                                ),
                                bytes7,
                            )
                        };
                        result8.push(e8);
                    }
                    _rt::cabi_dealloc(
                        base8,
                        len8 * (3 * ::core::mem::size_of::<*const u8>()),
                        ::core::mem::size_of::<*const u8>(),
                    );
                    let result9 = result8;
                    result9
                }
            }
        }
    }
    pub mod io {
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod error {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            /// A resource which represents some error information.
            ///
            /// The only method provided by this resource is `to-debug-string`,
            /// which provides some human-readable information about the error.
            ///
            /// In the `wasi:io` package, this resource is returned through the
            /// `wasi:io/streams/stream-error` type.
            ///
            /// To provide more specific error information, other interfaces may
            /// offer functions to "downcast" this error into more specific types. For example,
            /// errors returned from streams derived from filesystem types can be described using
            /// the filesystem's own error-code type. This is done using the function
            /// `wasi:filesystem/types/filesystem-error-code`, which takes a `borrow<error>`
            /// parameter and returns an `option<wasi:filesystem/types/error-code>`.
            ///
            /// The set of functions which can "downcast" an `error` into a more
            /// concrete type is open.
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct Error {
                handle: _rt::Resource<Error>,
            }
            impl Error {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for Error {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:io/error@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]error"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl Error {
                #[allow(unused_unsafe, clippy::all)]
                /// Returns a string that is suitable to assist humans in debugging
                /// this error.
                ///
                /// WARNING: The returned string should not be consumed mechanically!
                /// It may change across platforms, hosts, or other implementation
                /// details. Parsing this string is a major platform-compatibility
                /// hazard.
                #[allow(async_fn_in_trait)]
                pub fn to_debug_string(&self) -> _rt::Vec<u8> {
                    unsafe {
                        #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                        #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                        struct RetArea(
                            [::core::mem::MaybeUninit<
                                u8,
                            >; 2 * ::core::mem::size_of::<*const u8>()],
                        );
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2
                                * ::core::mem::size_of::<*const u8>()],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/error@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]error.to-debug-string"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = *ptr0.add(0).cast::<*mut u8>();
                        let l3 = *ptr0
                            .add(::core::mem::size_of::<*const u8>())
                            .cast::<usize>();
                        let len4 = l3;
                        let bytes4 = _rt::Vec::from_raw_parts(l2.cast(), len4, len4);
                        let result5 = bytes4;
                        result5
                    }
                }
            }
        }
        /// A poll API intended to let users wait for I/O events on multiple handles
        /// at once.
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod poll {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            /// `pollable` represents a single I/O event which may be ready, or not.
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct Pollable {
                handle: _rt::Resource<Pollable>,
            }
            impl Pollable {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for Pollable {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:io/poll@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]pollable"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl Pollable {
                #[allow(unused_unsafe, clippy::all)]
                /// Return the readiness of a pollable. This function never blocks.
                ///
                /// Returns `true` when the pollable is ready, and `false` otherwise.
                #[allow(async_fn_in_trait)]
                pub fn ready(&self) -> bool {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/poll@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]pollable.ready"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        _rt::bool_lift(ret as u8)
                    }
                }
            }
            impl Pollable {
                #[allow(unused_unsafe, clippy::all)]
                /// `block` returns immediately if the pollable is ready, and otherwise
                /// blocks until ready.
                ///
                /// This function is equivalent to calling `poll.poll` on a list
                /// containing only this pollable.
                #[allow(async_fn_in_trait)]
                pub fn block(&self) -> () {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/poll@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]pollable.block"]
                            fn wit_import0(_: i32);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) {
                            unreachable!()
                        }
                        wit_import0((self).handle() as i32);
                    }
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            /// Poll for completion on a set of pollables.
            ///
            /// This function takes a list of pollables, which identify I/O sources of
            /// interest, and waits until one or more of the events is ready for I/O.
            ///
            /// The result `list<u32>` contains one or more indices of handles in the
            /// argument list that is ready for I/O.
            ///
            /// This function traps if either:
            /// - the list is empty, or:
            /// - the list contains more elements than can be indexed with a `u32` value.
            ///
            /// A timeout can be implemented by adding a pollable from the
            /// wasi-clocks API to the list.
            ///
            /// This function does not return a `result`; polling in itself does not
            /// do any I/O so it doesn't fail. If any of the I/O sources identified by
            /// the pollables has an error, it is indicated by marking the source as
            /// being ready for I/O.
            #[allow(async_fn_in_trait)]
            pub fn poll(in_: &[&Pollable]) -> _rt::Vec<u32> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 2 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 2
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let vec0 = in_;
                    let len0 = vec0.len();
                    let layout0 = _rt::alloc::Layout::from_size_align(vec0.len() * 4, 4)
                        .unwrap();
                    let (result0, _cleanup0) = wit_bindgen::rt::Cleanup::new(layout0);
                    for (i, e) in vec0.into_iter().enumerate() {
                        let base = result0.add(i * 4);
                        {
                            *base.add(0).cast::<i32>() = (e).handle() as i32;
                        }
                    }
                    let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:io/poll@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "poll"]
                        fn wit_import2(_: *mut u8, _: usize, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import2(_: *mut u8, _: usize, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import2(result0, len0, ptr1);
                    let l3 = *ptr1.add(0).cast::<*mut u8>();
                    let l4 = *ptr1
                        .add(::core::mem::size_of::<*const u8>())
                        .cast::<usize>();
                    let len5 = l4;
                    let result6 = _rt::Vec::from_raw_parts(l3.cast(), len5, len5);
                    result6
                }
            }
        }
        /// WASI I/O is an I/O abstraction API which is currently focused on providing
        /// stream types.
        ///
        /// In the future, the component model is expected to add built-in stream types;
        /// when it does, they are expected to subsume this API.
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod streams {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Error = super::super::super::wasi::io::error::Error;
            pub type Pollable = super::super::super::wasi::io::poll::Pollable;
            /// An error for input-stream and output-stream operations.
            pub enum StreamError {
                /// The last operation (a write or flush) failed before completion.
                ///
                /// More information is available in the `error` payload.
                ///
                /// After this, the stream will be closed. All future operations return
                /// `stream-error::closed`.
                LastOperationFailed(Error),
                /// The stream is closed: no more input will be accepted by the
                /// stream. A closed output-stream will return this error on all
                /// future operations.
                Closed,
            }
            impl ::core::fmt::Debug for StreamError {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        StreamError::LastOperationFailed(e) => {
                            f.debug_tuple("StreamError::LastOperationFailed")
                                .field(e)
                                .finish()
                        }
                        StreamError::Closed => {
                            f.debug_tuple("StreamError::Closed").finish()
                        }
                    }
                }
            }
            impl ::core::fmt::Display for StreamError {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    write!(f, "{:?}", self)
                }
            }
            impl std::error::Error for StreamError {}
            /// An input bytestream.
            ///
            /// `input-stream`s are *non-blocking* to the extent practical on underlying
            /// platforms. I/O operations always return promptly; if fewer bytes are
            /// promptly available than requested, they return the number of bytes promptly
            /// available, which could even be zero. To wait for data to be available,
            /// use the `subscribe` function to obtain a `pollable` which can be polled
            /// for using `wasi:io/poll`.
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct InputStream {
                handle: _rt::Resource<InputStream>,
            }
            impl InputStream {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for InputStream {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]input-stream"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            /// An output bytestream.
            ///
            /// `output-stream`s are *non-blocking* to the extent practical on
            /// underlying platforms. Except where specified otherwise, I/O operations also
            /// always return promptly, after the number of bytes that can be written
            /// promptly, which could even be zero. To wait for the stream to be ready to
            /// accept data, the `subscribe` function to obtain a `pollable` which can be
            /// polled for using `wasi:io/poll`.
            ///
            /// Dropping an `output-stream` while there's still an active write in
            /// progress may result in the data being lost. Before dropping the stream,
            /// be sure to fully flush your writes.
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct OutputStream {
                handle: _rt::Resource<OutputStream>,
            }
            impl OutputStream {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for OutputStream {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]output-stream"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl InputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Perform a non-blocking read from the stream.
                ///
                /// When the source of a `read` is binary data, the bytes from the source
                /// are returned verbatim. When the source of a `read` is known to the
                /// implementation to be text, bytes containing the UTF-8 encoding of the
                /// text are returned.
                ///
                /// This function returns a list of bytes containing the read data,
                /// when successful. The returned list will contain up to `len` bytes;
                /// it may return fewer than requested, but not more. The list is
                /// empty when no bytes are available for reading at this time. The
                /// pollable given by `subscribe` will be ready when more bytes are
                /// available.
                ///
                /// This function fails with a `stream-error` when the operation
                /// encounters an error, giving `last-operation-failed`, or when the
                /// stream is closed, giving `closed`.
                ///
                /// When the caller gives a `len` of 0, it represents a request to
                /// read 0 bytes. If the stream is still open, this call should
                /// succeed and return an empty list, or otherwise fail with `closed`.
                ///
                /// The `len` parameter is a `u64`, which could represent a list of u8 which
                /// is not possible to allocate in wasm32, or not desirable to allocate as
                /// as a return value by the callee. The callee may return a list of bytes
                /// less than `len` in size while more bytes are available for reading.
                #[allow(async_fn_in_trait)]
                pub fn read(&self, len: u64) -> Result<_rt::Vec<u8>, StreamError> {
                    unsafe {
                        #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                        #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                        struct RetArea(
                            [::core::mem::MaybeUninit<
                                u8,
                            >; 3 * ::core::mem::size_of::<*const u8>()],
                        );
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 3
                                * ::core::mem::size_of::<*const u8>()],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]input-stream.read"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result9 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0
                                        .add(::core::mem::size_of::<*const u8>())
                                        .cast::<*mut u8>();
                                    let l4 = *ptr0
                                        .add(2 * ::core::mem::size_of::<*const u8>())
                                        .cast::<usize>();
                                    let len5 = l4;
                                    _rt::Vec::from_raw_parts(l3.cast(), len5, len5)
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l6 = i32::from(
                                        *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                    );
                                    let v8 = match l6 {
                                        0 => {
                                            let e8 = {
                                                let l7 = *ptr0
                                                    .add(4 + 1 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l7 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e8)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v8
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result9
                    }
                }
            }
            impl InputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Read bytes from a stream, after blocking until at least one byte can
                /// be read. Except for blocking, behavior is identical to `read`.
                #[allow(async_fn_in_trait)]
                pub fn blocking_read(
                    &self,
                    len: u64,
                ) -> Result<_rt::Vec<u8>, StreamError> {
                    unsafe {
                        #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                        #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                        struct RetArea(
                            [::core::mem::MaybeUninit<
                                u8,
                            >; 3 * ::core::mem::size_of::<*const u8>()],
                        );
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 3
                                * ::core::mem::size_of::<*const u8>()],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]input-stream.blocking-read"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result9 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0
                                        .add(::core::mem::size_of::<*const u8>())
                                        .cast::<*mut u8>();
                                    let l4 = *ptr0
                                        .add(2 * ::core::mem::size_of::<*const u8>())
                                        .cast::<usize>();
                                    let len5 = l4;
                                    _rt::Vec::from_raw_parts(l3.cast(), len5, len5)
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l6 = i32::from(
                                        *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                    );
                                    let v8 = match l6 {
                                        0 => {
                                            let e8 = {
                                                let l7 = *ptr0
                                                    .add(4 + 1 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l7 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e8)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v8
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result9
                    }
                }
            }
            impl InputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Skip bytes from a stream. Returns number of bytes skipped.
                ///
                /// Behaves identical to `read`, except instead of returning a list
                /// of bytes, returns the number of bytes consumed from the stream.
                #[allow(async_fn_in_trait)]
                pub fn skip(&self, len: u64) -> Result<u64, StreamError> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]input-stream.skip"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result7 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    let v6 = match l4 {
                                        0 => {
                                            let e6 = {
                                                let l5 = *ptr0.add(12).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l5 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e6)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v6
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl InputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Skip bytes from a stream, after blocking until at least one byte
                /// can be skipped. Except for blocking behavior, identical to `skip`.
                #[allow(async_fn_in_trait)]
                pub fn blocking_skip(&self, len: u64) -> Result<u64, StreamError> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]input-stream.blocking-skip"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result7 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    let v6 = match l4 {
                                        0 => {
                                            let e6 = {
                                                let l5 = *ptr0.add(12).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l5 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e6)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v6
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl InputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Create a `pollable` which will resolve once either the specified stream
                /// has bytes available to read or the other end of the stream has been
                /// closed.
                /// The created `pollable` is a child resource of the `input-stream`.
                /// Implementations may trap if the `input-stream` is dropped before
                /// all derived `pollable`s created with this function are dropped.
                #[allow(async_fn_in_trait)]
                pub fn subscribe(&self) -> Pollable {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]input-stream.subscribe"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::io::poll::Pollable::from_handle(
                            ret as u32,
                        )
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Check readiness for writing. This function never blocks.
                ///
                /// Returns the number of bytes permitted for the next call to `write`,
                /// or an error. Calling `write` with more bytes than this function has
                /// permitted will trap.
                ///
                /// When this function returns 0 bytes, the `subscribe` pollable will
                /// become ready when this function will report at least 1 byte, or an
                /// error.
                #[allow(async_fn_in_trait)]
                pub fn check_write(&self) -> Result<u64, StreamError> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.check-write"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result7 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    let v6 = match l4 {
                                        0 => {
                                            let e6 = {
                                                let l5 = *ptr0.add(12).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l5 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e6)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v6
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Perform a write. This function never blocks.
                ///
                /// When the destination of a `write` is binary data, the bytes from
                /// `contents` are written verbatim. When the destination of a `write` is
                /// known to the implementation to be text, the bytes of `contents` are
                /// transcoded from UTF-8 into the encoding of the destination and then
                /// written.
                ///
                /// Precondition: check-write gave permit of Ok(n) and contents has a
                /// length of less than or equal to n. Otherwise, this function will trap.
                ///
                /// returns Err(closed) without writing if the stream has closed since
                /// the last call to check-write provided a permit.
                #[allow(async_fn_in_trait)]
                pub fn write(&self, contents: &[u8]) -> Result<(), StreamError> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let vec0 = contents;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.write"]
                            fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import2(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import2((self).handle() as i32, ptr0.cast_mut(), len0, ptr1);
                        let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                        let result7 = match l3 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr1.add(4).cast::<u8>());
                                    let v6 = match l4 {
                                        0 => {
                                            let e6 = {
                                                let l5 = *ptr1.add(8).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l5 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e6)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v6
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Perform a write of up to 4096 bytes, and then flush the stream. Block
                /// until all of these operations are complete, or an error occurs.
                ///
                /// This is a convenience wrapper around the use of `check-write`,
                /// `subscribe`, `write`, and `flush`, and is implemented with the
                /// following pseudo-code:
                ///
                /// ```text
                /// let pollable = this.subscribe();
                /// while !contents.is_empty() {
                ///     // Wait for the stream to become writable
                ///     pollable.block();
                ///     let Ok(n) = this.check-write(); // eliding error handling
                ///     let len = min(n, contents.len());
                ///     let (chunk, rest) = contents.split_at(len);
                ///     this.write(chunk  );            // eliding error handling
                ///     contents = rest;
                /// }
                /// this.flush();
                /// // Wait for completion of `flush`
                /// pollable.block();
                /// // Check for any errors that arose during `flush`
                /// let _ = this.check-write();         // eliding error handling
                /// ```
                #[allow(async_fn_in_trait)]
                pub fn blocking_write_and_flush(
                    &self,
                    contents: &[u8],
                ) -> Result<(), StreamError> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let vec0 = contents;
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.blocking-write-and-flush"]
                            fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import2(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import2((self).handle() as i32, ptr0.cast_mut(), len0, ptr1);
                        let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                        let result7 = match l3 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr1.add(4).cast::<u8>());
                                    let v6 = match l4 {
                                        0 => {
                                            let e6 = {
                                                let l5 = *ptr1.add(8).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l5 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e6)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v6
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Request to flush buffered output. This function never blocks.
                ///
                /// This tells the output-stream that the caller intends any buffered
                /// output to be flushed. the output which is expected to be flushed
                /// is all that has been passed to `write` prior to this call.
                ///
                /// Upon calling this function, the `output-stream` will not accept any
                /// writes (`check-write` will return `ok(0)`) until the flush has
                /// completed. The `subscribe` pollable will become ready when the
                /// flush has completed and the stream can accept more writes.
                #[allow(async_fn_in_trait)]
                pub fn flush(&self) -> Result<(), StreamError> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.flush"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result6 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    let v5 = match l3 {
                                        0 => {
                                            let e5 = {
                                                let l4 = *ptr0.add(8).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l4 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e5)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v5
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Request to flush buffered output, and block until flush completes
                /// and stream is ready for writing again.
                #[allow(async_fn_in_trait)]
                pub fn blocking_flush(&self) -> Result<(), StreamError> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.blocking-flush"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result6 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    let v5 = match l3 {
                                        0 => {
                                            let e5 = {
                                                let l4 = *ptr0.add(8).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l4 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e5)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v5
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Create a `pollable` which will resolve once the output-stream
                /// is ready for more writing, or an error has occurred. When this
                /// pollable is ready, `check-write` will return `ok(n)` with n>0, or an
                /// error.
                ///
                /// If the stream is closed, this pollable is always ready immediately.
                ///
                /// The created `pollable` is a child resource of the `output-stream`.
                /// Implementations may trap if the `output-stream` is dropped before
                /// all derived `pollable`s created with this function are dropped.
                #[allow(async_fn_in_trait)]
                pub fn subscribe(&self) -> Pollable {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.subscribe"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::io::poll::Pollable::from_handle(
                            ret as u32,
                        )
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Write zeroes to a stream.
                ///
                /// This should be used precisely like `write` with the exact same
                /// preconditions (must use check-write first), but instead of
                /// passing a list of bytes, you simply pass the number of zero-bytes
                /// that should be written.
                #[allow(async_fn_in_trait)]
                pub fn write_zeroes(&self, len: u64) -> Result<(), StreamError> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.write-zeroes"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result6 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    let v5 = match l3 {
                                        0 => {
                                            let e5 = {
                                                let l4 = *ptr0.add(8).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l4 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e5)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v5
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Perform a write of up to 4096 zeroes, and then flush the stream.
                /// Block until all of these operations are complete, or an error
                /// occurs.
                ///
                /// This is a convenience wrapper around the use of `check-write`,
                /// `subscribe`, `write-zeroes`, and `flush`, and is implemented with
                /// the following pseudo-code:
                ///
                /// ```text
                /// let pollable = this.subscribe();
                /// while num_zeroes != 0 {
                ///     // Wait for the stream to become writable
                ///     pollable.block();
                ///     let Ok(n) = this.check-write(); // eliding error handling
                ///     let len = min(n, num_zeroes);
                ///     this.write-zeroes(len);         // eliding error handling
                ///     num_zeroes -= len;
                /// }
                /// this.flush();
                /// // Wait for completion of `flush`
                /// pollable.block();
                /// // Check for any errors that arose during `flush`
                /// let _ = this.check-write();         // eliding error handling
                /// ```
                #[allow(async_fn_in_trait)]
                pub fn blocking_write_zeroes_and_flush(
                    &self,
                    len: u64,
                ) -> Result<(), StreamError> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.blocking-write-zeroes-and-flush"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result6 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    let v5 = match l3 {
                                        0 => {
                                            let e5 = {
                                                let l4 = *ptr0.add(8).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l4 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e5)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v5
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Read from one stream and write to another.
                ///
                /// The behavior of splice is equivalent to:
                /// 1. calling `check-write` on the `output-stream`
                /// 2. calling `read` on the `input-stream` with the smaller of the
                /// `check-write` permitted length and the `len` provided to `splice`
                /// 3. calling `write` on the `output-stream` with that read data.
                ///
                /// Any error reported by the call to `check-write`, `read`, or
                /// `write` ends the splice and reports that error.
                ///
                /// This function returns the number of bytes transferred; it may be less
                /// than `len`.
                #[allow(async_fn_in_trait)]
                pub fn splice(
                    &self,
                    src: &InputStream,
                    len: u64,
                ) -> Result<u64, StreamError> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.splice"]
                            fn wit_import1(_: i32, _: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(
                            _: i32,
                            _: i32,
                            _: i64,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import1(
                            (self).handle() as i32,
                            (src).handle() as i32,
                            _rt::as_i64(&len),
                            ptr0,
                        );
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result7 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    let v6 = match l4 {
                                        0 => {
                                            let e6 = {
                                                let l5 = *ptr0.add(12).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l5 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e6)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v6
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl OutputStream {
                #[allow(unused_unsafe, clippy::all)]
                /// Read from one stream and write to another, with blocking.
                ///
                /// This is similar to `splice`, except that it blocks until the
                /// `output-stream` is ready for writing, and the `input-stream`
                /// is ready for reading, before performing the `splice`.
                #[allow(async_fn_in_trait)]
                pub fn blocking_splice(
                    &self,
                    src: &InputStream,
                    len: u64,
                ) -> Result<u64, StreamError> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:io/streams@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]output-stream.blocking-splice"]
                            fn wit_import1(_: i32, _: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(
                            _: i32,
                            _: i32,
                            _: i64,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import1(
                            (self).handle() as i32,
                            (src).handle() as i32,
                            _rt::as_i64(&len),
                            ptr0,
                        );
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result7 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    let v6 = match l4 {
                                        0 => {
                                            let e6 = {
                                                let l5 = *ptr0.add(12).cast::<i32>();
                                                super::super::super::wasi::io::error::Error::from_handle(
                                                    l5 as u32,
                                                )
                                            };
                                            StreamError::LastOperationFailed(e6)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            StreamError::Closed
                                        }
                                    };
                                    v6
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
        }
    }
    pub mod random {
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod random {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_random_bytes(len: u64) -> _rt::Vec<u8> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 2 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 2
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:random/random@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-random-bytes"]
                        fn wit_import1(_: i64, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i64, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(_rt::as_i64(&len), ptr0);
                    let l2 = *ptr0.add(0).cast::<*mut u8>();
                    let l3 = *ptr0
                        .add(::core::mem::size_of::<*const u8>())
                        .cast::<usize>();
                    let len4 = l3;
                    let result5 = _rt::Vec::from_raw_parts(l2.cast(), len4, len4);
                    result5
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_random_u64() -> u64 {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:random/random@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-random-u64"]
                        fn wit_import0() -> i64;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i64 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    ret as u64
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod insecure {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_insecure_random_bytes(len: u64) -> _rt::Vec<u8> {
                unsafe {
                    #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                    #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                    struct RetArea(
                        [::core::mem::MaybeUninit<
                            u8,
                        >; 2 * ::core::mem::size_of::<*const u8>()],
                    );
                    let mut ret_area = RetArea(
                        [::core::mem::MaybeUninit::uninit(); 2
                            * ::core::mem::size_of::<*const u8>()],
                    );
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:random/insecure@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-insecure-random-bytes"]
                        fn wit_import1(_: i64, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i64, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(_rt::as_i64(&len), ptr0);
                    let l2 = *ptr0.add(0).cast::<*mut u8>();
                    let l3 = *ptr0
                        .add(::core::mem::size_of::<*const u8>())
                        .cast::<usize>();
                    let len4 = l3;
                    let result5 = _rt::Vec::from_raw_parts(l2.cast(), len4, len4);
                    result5
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn get_insecure_random_u64() -> u64 {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:random/insecure@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "get-insecure-random-u64"]
                        fn wit_import0() -> i64;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i64 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    ret as u64
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod insecure_seed {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn insecure_seed() -> (u64, u64) {
                unsafe {
                    #[repr(align(8))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:random/insecure-seed@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "insecure-seed"]
                        fn wit_import1(_: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(ptr0);
                    let l2 = *ptr0.add(0).cast::<i64>();
                    let l3 = *ptr0.add(8).cast::<i64>();
                    let result4 = (l2 as u64, l3 as u64);
                    result4
                }
            }
        }
    }
    pub mod sockets {
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod network {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Error = super::super::super::wasi::io::error::Error;
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct Network {
                handle: _rt::Resource<Network>,
            }
            impl Network {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for Network {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/network@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]network"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum ErrorCode {
                Unknown,
                AccessDenied,
                NotSupported,
                InvalidArgument,
                OutOfMemory,
                Timeout,
                ConcurrencyConflict,
                NotInProgress,
                WouldBlock,
                InvalidState,
                NewSocketLimit,
                AddressNotBindable,
                AddressInUse,
                RemoteUnreachable,
                ConnectionRefused,
                ConnectionReset,
                ConnectionAborted,
                DatagramTooLarge,
                NameUnresolvable,
                TemporaryResolverFailure,
                PermanentResolverFailure,
            }
            impl ErrorCode {
                pub fn name(&self) -> &'static str {
                    match self {
                        ErrorCode::Unknown => "unknown",
                        ErrorCode::AccessDenied => "access-denied",
                        ErrorCode::NotSupported => "not-supported",
                        ErrorCode::InvalidArgument => "invalid-argument",
                        ErrorCode::OutOfMemory => "out-of-memory",
                        ErrorCode::Timeout => "timeout",
                        ErrorCode::ConcurrencyConflict => "concurrency-conflict",
                        ErrorCode::NotInProgress => "not-in-progress",
                        ErrorCode::WouldBlock => "would-block",
                        ErrorCode::InvalidState => "invalid-state",
                        ErrorCode::NewSocketLimit => "new-socket-limit",
                        ErrorCode::AddressNotBindable => "address-not-bindable",
                        ErrorCode::AddressInUse => "address-in-use",
                        ErrorCode::RemoteUnreachable => "remote-unreachable",
                        ErrorCode::ConnectionRefused => "connection-refused",
                        ErrorCode::ConnectionReset => "connection-reset",
                        ErrorCode::ConnectionAborted => "connection-aborted",
                        ErrorCode::DatagramTooLarge => "datagram-too-large",
                        ErrorCode::NameUnresolvable => "name-unresolvable",
                        ErrorCode::TemporaryResolverFailure => {
                            "temporary-resolver-failure"
                        }
                        ErrorCode::PermanentResolverFailure => {
                            "permanent-resolver-failure"
                        }
                    }
                }
                pub fn message(&self) -> &'static str {
                    match self {
                        ErrorCode::Unknown => "",
                        ErrorCode::AccessDenied => "",
                        ErrorCode::NotSupported => "",
                        ErrorCode::InvalidArgument => "",
                        ErrorCode::OutOfMemory => "",
                        ErrorCode::Timeout => "",
                        ErrorCode::ConcurrencyConflict => "",
                        ErrorCode::NotInProgress => "",
                        ErrorCode::WouldBlock => "",
                        ErrorCode::InvalidState => "",
                        ErrorCode::NewSocketLimit => "",
                        ErrorCode::AddressNotBindable => "",
                        ErrorCode::AddressInUse => "",
                        ErrorCode::RemoteUnreachable => "",
                        ErrorCode::ConnectionRefused => "",
                        ErrorCode::ConnectionReset => "",
                        ErrorCode::ConnectionAborted => "",
                        ErrorCode::DatagramTooLarge => "",
                        ErrorCode::NameUnresolvable => "",
                        ErrorCode::TemporaryResolverFailure => "",
                        ErrorCode::PermanentResolverFailure => "",
                    }
                }
            }
            impl ::core::fmt::Debug for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("ErrorCode")
                        .field("code", &(*self as i32))
                        .field("name", &self.name())
                        .field("message", &self.message())
                        .finish()
                }
            }
            impl ::core::fmt::Display for ErrorCode {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    write!(f, "{} (error {})", self.name(), * self as i32)
                }
            }
            impl std::error::Error for ErrorCode {}
            impl ErrorCode {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> ErrorCode {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => ErrorCode::Unknown,
                        1 => ErrorCode::AccessDenied,
                        2 => ErrorCode::NotSupported,
                        3 => ErrorCode::InvalidArgument,
                        4 => ErrorCode::OutOfMemory,
                        5 => ErrorCode::Timeout,
                        6 => ErrorCode::ConcurrencyConflict,
                        7 => ErrorCode::NotInProgress,
                        8 => ErrorCode::WouldBlock,
                        9 => ErrorCode::InvalidState,
                        10 => ErrorCode::NewSocketLimit,
                        11 => ErrorCode::AddressNotBindable,
                        12 => ErrorCode::AddressInUse,
                        13 => ErrorCode::RemoteUnreachable,
                        14 => ErrorCode::ConnectionRefused,
                        15 => ErrorCode::ConnectionReset,
                        16 => ErrorCode::ConnectionAborted,
                        17 => ErrorCode::DatagramTooLarge,
                        18 => ErrorCode::NameUnresolvable,
                        19 => ErrorCode::TemporaryResolverFailure,
                        20 => ErrorCode::PermanentResolverFailure,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum IpAddressFamily {
                Ipv4,
                Ipv6,
            }
            impl ::core::fmt::Debug for IpAddressFamily {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        IpAddressFamily::Ipv4 => {
                            f.debug_tuple("IpAddressFamily::Ipv4").finish()
                        }
                        IpAddressFamily::Ipv6 => {
                            f.debug_tuple("IpAddressFamily::Ipv6").finish()
                        }
                    }
                }
            }
            impl IpAddressFamily {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> IpAddressFamily {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => IpAddressFamily::Ipv4,
                        1 => IpAddressFamily::Ipv6,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            pub type Ipv4Address = (u8, u8, u8, u8);
            pub type Ipv6Address = (u16, u16, u16, u16, u16, u16, u16, u16);
            #[derive(Clone, Copy)]
            pub enum IpAddress {
                Ipv4(Ipv4Address),
                Ipv6(Ipv6Address),
            }
            impl ::core::fmt::Debug for IpAddress {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        IpAddress::Ipv4(e) => {
                            f.debug_tuple("IpAddress::Ipv4").field(e).finish()
                        }
                        IpAddress::Ipv6(e) => {
                            f.debug_tuple("IpAddress::Ipv6").field(e).finish()
                        }
                    }
                }
            }
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct Ipv4SocketAddress {
                pub port: u16,
                pub address: Ipv4Address,
            }
            impl ::core::fmt::Debug for Ipv4SocketAddress {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("Ipv4SocketAddress")
                        .field("port", &self.port)
                        .field("address", &self.address)
                        .finish()
                }
            }
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct Ipv6SocketAddress {
                pub port: u16,
                pub flow_info: u32,
                pub address: Ipv6Address,
                pub scope_id: u32,
            }
            impl ::core::fmt::Debug for Ipv6SocketAddress {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("Ipv6SocketAddress")
                        .field("port", &self.port)
                        .field("flow-info", &self.flow_info)
                        .field("address", &self.address)
                        .field("scope-id", &self.scope_id)
                        .finish()
                }
            }
            #[derive(Clone, Copy)]
            pub enum IpSocketAddress {
                Ipv4(Ipv4SocketAddress),
                Ipv6(Ipv6SocketAddress),
            }
            impl ::core::fmt::Debug for IpSocketAddress {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        IpSocketAddress::Ipv4(e) => {
                            f.debug_tuple("IpSocketAddress::Ipv4").field(e).finish()
                        }
                        IpSocketAddress::Ipv6(e) => {
                            f.debug_tuple("IpSocketAddress::Ipv6").field(e).finish()
                        }
                    }
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn network_error_code(err: &Error) -> Option<ErrorCode> {
                unsafe {
                    #[repr(align(1))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 2]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/network@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "network-error-code"]
                        fn wit_import1(_: i32, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1((err).handle() as i32, ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result4 = match l2 {
                        0 => None,
                        1 => {
                            let e = {
                                let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                ErrorCode::_lift(l3 as u8)
                            };
                            Some(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result4
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod instance_network {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            pub type Network = super::super::super::wasi::sockets::network::Network;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn instance_network() -> Network {
                unsafe {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/instance-network@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "instance-network"]
                        fn wit_import0() -> i32;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import0() -> i32 {
                        unreachable!()
                    }
                    let ret = wit_import0();
                    super::super::super::wasi::sockets::network::Network::from_handle(
                        ret as u32,
                    )
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod udp {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Pollable = super::super::super::wasi::io::poll::Pollable;
            pub type Network = super::super::super::wasi::sockets::network::Network;
            pub type ErrorCode = super::super::super::wasi::sockets::network::ErrorCode;
            pub type IpSocketAddress = super::super::super::wasi::sockets::network::IpSocketAddress;
            pub type IpAddressFamily = super::super::super::wasi::sockets::network::IpAddressFamily;
            #[derive(Clone)]
            pub struct IncomingDatagram {
                pub data: _rt::Vec<u8>,
                pub remote_address: IpSocketAddress,
            }
            impl ::core::fmt::Debug for IncomingDatagram {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("IncomingDatagram")
                        .field("data", &self.data)
                        .field("remote-address", &self.remote_address)
                        .finish()
                }
            }
            #[derive(Clone)]
            pub struct OutgoingDatagram<'a> {
                pub data: &'a [u8],
                pub remote_address: Option<IpSocketAddress>,
            }
            impl<'a> ::core::fmt::Debug for OutgoingDatagram<'a> {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    f.debug_struct("OutgoingDatagram")
                        .field("data", &self.data)
                        .field("remote-address", &self.remote_address)
                        .finish()
                }
            }
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct UdpSocket {
                handle: _rt::Resource<UdpSocket>,
            }
            impl UdpSocket {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for UdpSocket {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]udp-socket"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct IncomingDatagramStream {
                handle: _rt::Resource<IncomingDatagramStream>,
            }
            impl IncomingDatagramStream {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for IncomingDatagramStream {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]incoming-datagram-stream"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct OutgoingDatagramStream {
                handle: _rt::Resource<OutgoingDatagramStream>,
            }
            impl OutgoingDatagramStream {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for OutgoingDatagramStream {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]outgoing-datagram-stream"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn start_bind(
                    &self,
                    network: &Network,
                    local_address: IpSocketAddress,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        use super::super::super::wasi::sockets::network::IpSocketAddress as V4;
                        let (
                            result5_0,
                            result5_1,
                            result5_2,
                            result5_3,
                            result5_4,
                            result5_5,
                            result5_6,
                            result5_7,
                            result5_8,
                            result5_9,
                            result5_10,
                            result5_11,
                        ) = match local_address {
                            V4::Ipv4(e) => {
                                let super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                    port: port0,
                                    address: address0,
                                } = e;
                                let (t1_0, t1_1, t1_2, t1_3) = address0;
                                (
                                    0i32,
                                    _rt::as_i32(port0),
                                    _rt::as_i32(t1_0),
                                    _rt::as_i32(t1_1),
                                    _rt::as_i32(t1_2),
                                    _rt::as_i32(t1_3),
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                )
                            }
                            V4::Ipv6(e) => {
                                let super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                    port: port2,
                                    flow_info: flow_info2,
                                    address: address2,
                                    scope_id: scope_id2,
                                } = e;
                                let (t3_0, t3_1, t3_2, t3_3, t3_4, t3_5, t3_6, t3_7) = address2;
                                (
                                    1i32,
                                    _rt::as_i32(port2),
                                    _rt::as_i32(flow_info2),
                                    _rt::as_i32(t3_0),
                                    _rt::as_i32(t3_1),
                                    _rt::as_i32(t3_2),
                                    _rt::as_i32(t3_3),
                                    _rt::as_i32(t3_4),
                                    _rt::as_i32(t3_5),
                                    _rt::as_i32(t3_6),
                                    _rt::as_i32(t3_7),
                                    _rt::as_i32(scope_id2),
                                )
                            }
                        };
                        let ptr6 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.start-bind"]
                            fn wit_import7(
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import7(
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import7(
                            (self).handle() as i32,
                            (network).handle() as i32,
                            result5_0,
                            result5_1,
                            result5_2,
                            result5_3,
                            result5_4,
                            result5_5,
                            result5_6,
                            result5_7,
                            result5_8,
                            result5_9,
                            result5_10,
                            result5_11,
                            ptr6,
                        );
                        let l8 = i32::from(*ptr6.add(0).cast::<u8>());
                        let result10 = match l8 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l9 = i32::from(*ptr6.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l9 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result10
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn finish_bind(&self) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.finish-bind"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn stream(
                    &self,
                    remote_address: Option<IpSocketAddress>,
                ) -> Result<
                    (IncomingDatagramStream, OutgoingDatagramStream),
                    ErrorCode,
                > {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let (
                            result6_0,
                            result6_1,
                            result6_2,
                            result6_3,
                            result6_4,
                            result6_5,
                            result6_6,
                            result6_7,
                            result6_8,
                            result6_9,
                            result6_10,
                            result6_11,
                            result6_12,
                        ) = match remote_address {
                            Some(e) => {
                                use super::super::super::wasi::sockets::network::IpSocketAddress as V4;
                                let (
                                    result5_0,
                                    result5_1,
                                    result5_2,
                                    result5_3,
                                    result5_4,
                                    result5_5,
                                    result5_6,
                                    result5_7,
                                    result5_8,
                                    result5_9,
                                    result5_10,
                                    result5_11,
                                ) = match e {
                                    V4::Ipv4(e) => {
                                        let super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                            port: port0,
                                            address: address0,
                                        } = e;
                                        let (t1_0, t1_1, t1_2, t1_3) = address0;
                                        (
                                            0i32,
                                            _rt::as_i32(port0),
                                            _rt::as_i32(t1_0),
                                            _rt::as_i32(t1_1),
                                            _rt::as_i32(t1_2),
                                            _rt::as_i32(t1_3),
                                            0i32,
                                            0i32,
                                            0i32,
                                            0i32,
                                            0i32,
                                            0i32,
                                        )
                                    }
                                    V4::Ipv6(e) => {
                                        let super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                            port: port2,
                                            flow_info: flow_info2,
                                            address: address2,
                                            scope_id: scope_id2,
                                        } = e;
                                        let (t3_0, t3_1, t3_2, t3_3, t3_4, t3_5, t3_6, t3_7) = address2;
                                        (
                                            1i32,
                                            _rt::as_i32(port2),
                                            _rt::as_i32(flow_info2),
                                            _rt::as_i32(t3_0),
                                            _rt::as_i32(t3_1),
                                            _rt::as_i32(t3_2),
                                            _rt::as_i32(t3_3),
                                            _rt::as_i32(t3_4),
                                            _rt::as_i32(t3_5),
                                            _rt::as_i32(t3_6),
                                            _rt::as_i32(t3_7),
                                            _rt::as_i32(scope_id2),
                                        )
                                    }
                                };
                                (
                                    1i32,
                                    result5_0,
                                    result5_1,
                                    result5_2,
                                    result5_3,
                                    result5_4,
                                    result5_5,
                                    result5_6,
                                    result5_7,
                                    result5_8,
                                    result5_9,
                                    result5_10,
                                    result5_11,
                                )
                            }
                            None => {
                                (
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                )
                            }
                        };
                        let ptr7 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.stream"]
                            fn wit_import8(
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import8(
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import8(
                            (self).handle() as i32,
                            result6_0,
                            result6_1,
                            result6_2,
                            result6_3,
                            result6_4,
                            result6_5,
                            result6_6,
                            result6_7,
                            result6_8,
                            result6_9,
                            result6_10,
                            result6_11,
                            result6_12,
                            ptr7,
                        );
                        let l9 = i32::from(*ptr7.add(0).cast::<u8>());
                        let result13 = match l9 {
                            0 => {
                                let e = {
                                    let l10 = *ptr7.add(4).cast::<i32>();
                                    let l11 = *ptr7.add(8).cast::<i32>();
                                    (
                                        IncomingDatagramStream::from_handle(l10 as u32),
                                        OutgoingDatagramStream::from_handle(l11 as u32),
                                    )
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l12 = i32::from(*ptr7.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l12 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result13
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn local_address(&self) -> Result<IpSocketAddress, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 36]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 36],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.local-address"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result22 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    use super::super::super::wasi::sockets::network::IpSocketAddress as V20;
                                    let v20 = match l3 {
                                        0 => {
                                            let e20 = {
                                                let l4 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l5 = i32::from(*ptr0.add(10).cast::<u8>());
                                                let l6 = i32::from(*ptr0.add(11).cast::<u8>());
                                                let l7 = i32::from(*ptr0.add(12).cast::<u8>());
                                                let l8 = i32::from(*ptr0.add(13).cast::<u8>());
                                                super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                                    port: l4 as u16,
                                                    address: (l5 as u8, l6 as u8, l7 as u8, l8 as u8),
                                                }
                                            };
                                            V20::Ipv4(e20)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            let e20 = {
                                                let l9 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l10 = *ptr0.add(12).cast::<i32>();
                                                let l11 = i32::from(*ptr0.add(16).cast::<u16>());
                                                let l12 = i32::from(*ptr0.add(18).cast::<u16>());
                                                let l13 = i32::from(*ptr0.add(20).cast::<u16>());
                                                let l14 = i32::from(*ptr0.add(22).cast::<u16>());
                                                let l15 = i32::from(*ptr0.add(24).cast::<u16>());
                                                let l16 = i32::from(*ptr0.add(26).cast::<u16>());
                                                let l17 = i32::from(*ptr0.add(28).cast::<u16>());
                                                let l18 = i32::from(*ptr0.add(30).cast::<u16>());
                                                let l19 = *ptr0.add(32).cast::<i32>();
                                                super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                                    port: l9 as u16,
                                                    flow_info: l10 as u32,
                                                    address: (
                                                        l11 as u16,
                                                        l12 as u16,
                                                        l13 as u16,
                                                        l14 as u16,
                                                        l15 as u16,
                                                        l16 as u16,
                                                        l17 as u16,
                                                        l18 as u16,
                                                    ),
                                                    scope_id: l19 as u32,
                                                }
                                            };
                                            V20::Ipv6(e20)
                                        }
                                    };
                                    v20
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l21 = i32::from(*ptr0.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l21 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result22
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn remote_address(&self) -> Result<IpSocketAddress, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 36]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 36],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.remote-address"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result22 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    use super::super::super::wasi::sockets::network::IpSocketAddress as V20;
                                    let v20 = match l3 {
                                        0 => {
                                            let e20 = {
                                                let l4 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l5 = i32::from(*ptr0.add(10).cast::<u8>());
                                                let l6 = i32::from(*ptr0.add(11).cast::<u8>());
                                                let l7 = i32::from(*ptr0.add(12).cast::<u8>());
                                                let l8 = i32::from(*ptr0.add(13).cast::<u8>());
                                                super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                                    port: l4 as u16,
                                                    address: (l5 as u8, l6 as u8, l7 as u8, l8 as u8),
                                                }
                                            };
                                            V20::Ipv4(e20)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            let e20 = {
                                                let l9 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l10 = *ptr0.add(12).cast::<i32>();
                                                let l11 = i32::from(*ptr0.add(16).cast::<u16>());
                                                let l12 = i32::from(*ptr0.add(18).cast::<u16>());
                                                let l13 = i32::from(*ptr0.add(20).cast::<u16>());
                                                let l14 = i32::from(*ptr0.add(22).cast::<u16>());
                                                let l15 = i32::from(*ptr0.add(24).cast::<u16>());
                                                let l16 = i32::from(*ptr0.add(26).cast::<u16>());
                                                let l17 = i32::from(*ptr0.add(28).cast::<u16>());
                                                let l18 = i32::from(*ptr0.add(30).cast::<u16>());
                                                let l19 = *ptr0.add(32).cast::<i32>();
                                                super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                                    port: l9 as u16,
                                                    flow_info: l10 as u32,
                                                    address: (
                                                        l11 as u16,
                                                        l12 as u16,
                                                        l13 as u16,
                                                        l14 as u16,
                                                        l15 as u16,
                                                        l16 as u16,
                                                        l17 as u16,
                                                        l18 as u16,
                                                    ),
                                                    scope_id: l19 as u32,
                                                }
                                            };
                                            V20::Ipv6(e20)
                                        }
                                    };
                                    v20
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l21 = i32::from(*ptr0.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l21 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result22
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn address_family(&self) -> IpAddressFamily {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.address-family"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::sockets::network::IpAddressFamily::_lift(
                            ret as u8,
                        )
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn unicast_hop_limit(&self) -> Result<u8, ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.unicast-hop-limit"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    l3 as u8
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_unicast_hop_limit(&self, value: u8) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.set-unicast-hop-limit"]
                            fn wit_import1(_: i32, _: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i32(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn receive_buffer_size(&self) -> Result<u64, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.receive-buffer-size"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_receive_buffer_size(
                    &self,
                    value: u64,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.set-receive-buffer-size"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn send_buffer_size(&self) -> Result<u64, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.send-buffer-size"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_send_buffer_size(&self, value: u64) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.set-send-buffer-size"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl UdpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn subscribe(&self) -> Pollable {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]udp-socket.subscribe"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::io::poll::Pollable::from_handle(
                            ret as u32,
                        )
                    }
                }
            }
            impl IncomingDatagramStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn receive(
                    &self,
                    max_results: u64,
                ) -> Result<_rt::Vec<IncomingDatagram>, ErrorCode> {
                    unsafe {
                        #[cfg_attr(target_pointer_width = "64", repr(align(8)))]
                        #[cfg_attr(target_pointer_width = "32", repr(align(4)))]
                        struct RetArea(
                            [::core::mem::MaybeUninit<
                                u8,
                            >; 3 * ::core::mem::size_of::<*const u8>()],
                        );
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 3
                                * ::core::mem::size_of::<*const u8>()],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]incoming-datagram-stream.receive"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1(
                            (self).handle() as i32,
                            _rt::as_i64(&max_results),
                            ptr0,
                        );
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result28 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0
                                        .add(::core::mem::size_of::<*const u8>())
                                        .cast::<*mut u8>();
                                    let l4 = *ptr0
                                        .add(2 * ::core::mem::size_of::<*const u8>())
                                        .cast::<usize>();
                                    let base26 = l3;
                                    let len26 = l4;
                                    let mut result26 = _rt::Vec::with_capacity(len26);
                                    for i in 0..len26 {
                                        let base = base26
                                            .add(i * (32 + 2 * ::core::mem::size_of::<*const u8>()));
                                        let e26 = {
                                            let l5 = *base.add(0).cast::<*mut u8>();
                                            let l6 = *base
                                                .add(::core::mem::size_of::<*const u8>())
                                                .cast::<usize>();
                                            let len7 = l6;
                                            let l8 = i32::from(
                                                *base
                                                    .add(2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u8>(),
                                            );
                                            use super::super::super::wasi::sockets::network::IpSocketAddress as V25;
                                            let v25 = match l8 {
                                                0 => {
                                                    let e25 = {
                                                        let l9 = i32::from(
                                                            *base
                                                                .add(4 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l10 = i32::from(
                                                            *base
                                                                .add(6 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u8>(),
                                                        );
                                                        let l11 = i32::from(
                                                            *base
                                                                .add(7 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u8>(),
                                                        );
                                                        let l12 = i32::from(
                                                            *base
                                                                .add(8 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u8>(),
                                                        );
                                                        let l13 = i32::from(
                                                            *base
                                                                .add(9 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u8>(),
                                                        );
                                                        super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                                            port: l9 as u16,
                                                            address: (l10 as u8, l11 as u8, l12 as u8, l13 as u8),
                                                        }
                                                    };
                                                    V25::Ipv4(e25)
                                                }
                                                n => {
                                                    debug_assert_eq!(n, 1, "invalid enum discriminant");
                                                    let e25 = {
                                                        let l14 = i32::from(
                                                            *base
                                                                .add(4 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l15 = *base
                                                            .add(8 + 2 * ::core::mem::size_of::<*const u8>())
                                                            .cast::<i32>();
                                                        let l16 = i32::from(
                                                            *base
                                                                .add(12 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l17 = i32::from(
                                                            *base
                                                                .add(14 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l18 = i32::from(
                                                            *base
                                                                .add(16 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l19 = i32::from(
                                                            *base
                                                                .add(18 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l20 = i32::from(
                                                            *base
                                                                .add(20 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l21 = i32::from(
                                                            *base
                                                                .add(22 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l22 = i32::from(
                                                            *base
                                                                .add(24 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l23 = i32::from(
                                                            *base
                                                                .add(26 + 2 * ::core::mem::size_of::<*const u8>())
                                                                .cast::<u16>(),
                                                        );
                                                        let l24 = *base
                                                            .add(28 + 2 * ::core::mem::size_of::<*const u8>())
                                                            .cast::<i32>();
                                                        super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                                            port: l14 as u16,
                                                            flow_info: l15 as u32,
                                                            address: (
                                                                l16 as u16,
                                                                l17 as u16,
                                                                l18 as u16,
                                                                l19 as u16,
                                                                l20 as u16,
                                                                l21 as u16,
                                                                l22 as u16,
                                                                l23 as u16,
                                                            ),
                                                            scope_id: l24 as u32,
                                                        }
                                                    };
                                                    V25::Ipv6(e25)
                                                }
                                            };
                                            IncomingDatagram {
                                                data: _rt::Vec::from_raw_parts(l5.cast(), len7, len7),
                                                remote_address: v25,
                                            }
                                        };
                                        result26.push(e26);
                                    }
                                    _rt::cabi_dealloc(
                                        base26,
                                        len26 * (32 + 2 * ::core::mem::size_of::<*const u8>()),
                                        ::core::mem::size_of::<*const u8>(),
                                    );
                                    result26
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l27 = i32::from(
                                        *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                                    );
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l27 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result28
                    }
                }
            }
            impl IncomingDatagramStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn subscribe(&self) -> Pollable {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]incoming-datagram-stream.subscribe"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::io::poll::Pollable::from_handle(
                            ret as u32,
                        )
                    }
                }
            }
            impl OutgoingDatagramStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn check_send(&self) -> Result<u64, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]outgoing-datagram-stream.check-send"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl OutgoingDatagramStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn send(
                    &self,
                    datagrams: &[OutgoingDatagram<'_>],
                ) -> Result<u64, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let vec7 = datagrams;
                        let len7 = vec7.len();
                        let layout7 = _rt::alloc::Layout::from_size_align(
                                vec7.len() * (32 + 3 * ::core::mem::size_of::<*const u8>()),
                                ::core::mem::size_of::<*const u8>(),
                            )
                            .unwrap();
                        let (result7, _cleanup7) = wit_bindgen::rt::Cleanup::new(
                            layout7,
                        );
                        for (i, e) in vec7.into_iter().enumerate() {
                            let base = result7
                                .add(i * (32 + 3 * ::core::mem::size_of::<*const u8>()));
                            {
                                let OutgoingDatagram {
                                    data: data0,
                                    remote_address: remote_address0,
                                } = e;
                                let vec1 = data0;
                                let ptr1 = vec1.as_ptr().cast::<u8>();
                                let len1 = vec1.len();
                                *base
                                    .add(::core::mem::size_of::<*const u8>())
                                    .cast::<usize>() = len1;
                                *base.add(0).cast::<*mut u8>() = ptr1.cast_mut();
                                match remote_address0 {
                                    Some(e) => {
                                        *base
                                            .add(2 * ::core::mem::size_of::<*const u8>())
                                            .cast::<u8>() = (1i32) as u8;
                                        use super::super::super::wasi::sockets::network::IpSocketAddress as V6;
                                        match e {
                                            V6::Ipv4(e) => {
                                                *base
                                                    .add(4 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u8>() = (0i32) as u8;
                                                let super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                                    port: port2,
                                                    address: address2,
                                                } = e;
                                                *base
                                                    .add(8 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(port2)) as u16;
                                                let (t3_0, t3_1, t3_2, t3_3) = address2;
                                                *base
                                                    .add(10 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u8>() = (_rt::as_i32(t3_0)) as u8;
                                                *base
                                                    .add(11 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u8>() = (_rt::as_i32(t3_1)) as u8;
                                                *base
                                                    .add(12 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u8>() = (_rt::as_i32(t3_2)) as u8;
                                                *base
                                                    .add(13 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u8>() = (_rt::as_i32(t3_3)) as u8;
                                            }
                                            V6::Ipv6(e) => {
                                                *base
                                                    .add(4 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u8>() = (1i32) as u8;
                                                let super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                                    port: port4,
                                                    flow_info: flow_info4,
                                                    address: address4,
                                                    scope_id: scope_id4,
                                                } = e;
                                                *base
                                                    .add(8 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(port4)) as u16;
                                                *base
                                                    .add(12 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<i32>() = _rt::as_i32(flow_info4);
                                                let (t5_0, t5_1, t5_2, t5_3, t5_4, t5_5, t5_6, t5_7) = address4;
                                                *base
                                                    .add(16 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_0)) as u16;
                                                *base
                                                    .add(18 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_1)) as u16;
                                                *base
                                                    .add(20 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_2)) as u16;
                                                *base
                                                    .add(22 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_3)) as u16;
                                                *base
                                                    .add(24 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_4)) as u16;
                                                *base
                                                    .add(26 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_5)) as u16;
                                                *base
                                                    .add(28 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_6)) as u16;
                                                *base
                                                    .add(30 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<u16>() = (_rt::as_i32(t5_7)) as u16;
                                                *base
                                                    .add(32 + 2 * ::core::mem::size_of::<*const u8>())
                                                    .cast::<i32>() = _rt::as_i32(scope_id4);
                                            }
                                        }
                                    }
                                    None => {
                                        *base
                                            .add(2 * ::core::mem::size_of::<*const u8>())
                                            .cast::<u8>() = (0i32) as u8;
                                    }
                                };
                            }
                        }
                        let ptr8 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]outgoing-datagram-stream.send"]
                            fn wit_import9(_: i32, _: *mut u8, _: usize, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import9(
                            _: i32,
                            _: *mut u8,
                            _: usize,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import9((self).handle() as i32, result7, len7, ptr8);
                        let l10 = i32::from(*ptr8.add(0).cast::<u8>());
                        let result13 = match l10 {
                            0 => {
                                let e = {
                                    let l11 = *ptr8.add(8).cast::<i64>();
                                    l11 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l12 = i32::from(*ptr8.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l12 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result13
                    }
                }
            }
            impl OutgoingDatagramStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn subscribe(&self) -> Pollable {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/udp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]outgoing-datagram-stream.subscribe"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::io::poll::Pollable::from_handle(
                            ret as u32,
                        )
                    }
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod udp_create_socket {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Network = super::super::super::wasi::sockets::network::Network;
            pub type ErrorCode = super::super::super::wasi::sockets::network::ErrorCode;
            pub type IpAddressFamily = super::super::super::wasi::sockets::network::IpAddressFamily;
            pub type UdpSocket = super::super::super::wasi::sockets::udp::UdpSocket;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn create_udp_socket(
                address_family: IpAddressFamily,
            ) -> Result<UdpSocket, ErrorCode> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/udp-create-socket@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "create-udp-socket"]
                        fn wit_import1(_: i32, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(address_family.clone() as i32, ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result5 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                super::super::super::wasi::sockets::udp::UdpSocket::from_handle(
                                    l3 as u32,
                                )
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                    l4 as u8,
                                )
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result5
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod tcp {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type InputStream = super::super::super::wasi::io::streams::InputStream;
            pub type OutputStream = super::super::super::wasi::io::streams::OutputStream;
            pub type Pollable = super::super::super::wasi::io::poll::Pollable;
            pub type Duration = super::super::super::wasi::clocks::monotonic_clock::Duration;
            pub type Network = super::super::super::wasi::sockets::network::Network;
            pub type ErrorCode = super::super::super::wasi::sockets::network::ErrorCode;
            pub type IpSocketAddress = super::super::super::wasi::sockets::network::IpSocketAddress;
            pub type IpAddressFamily = super::super::super::wasi::sockets::network::IpAddressFamily;
            #[repr(u8)]
            #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
            pub enum ShutdownType {
                Receive,
                Send,
                Both,
            }
            impl ::core::fmt::Debug for ShutdownType {
                fn fmt(
                    &self,
                    f: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        ShutdownType::Receive => {
                            f.debug_tuple("ShutdownType::Receive").finish()
                        }
                        ShutdownType::Send => {
                            f.debug_tuple("ShutdownType::Send").finish()
                        }
                        ShutdownType::Both => {
                            f.debug_tuple("ShutdownType::Both").finish()
                        }
                    }
                }
            }
            impl ShutdownType {
                #[doc(hidden)]
                pub unsafe fn _lift(val: u8) -> ShutdownType {
                    if !cfg!(debug_assertions) {
                        return unsafe { ::core::mem::transmute(val) };
                    }
                    match val {
                        0 => ShutdownType::Receive,
                        1 => ShutdownType::Send,
                        2 => ShutdownType::Both,
                        _ => panic!("invalid enum discriminant"),
                    }
                }
            }
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct TcpSocket {
                handle: _rt::Resource<TcpSocket>,
            }
            impl TcpSocket {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for TcpSocket {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]tcp-socket"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn start_bind(
                    &self,
                    network: &Network,
                    local_address: IpSocketAddress,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        use super::super::super::wasi::sockets::network::IpSocketAddress as V4;
                        let (
                            result5_0,
                            result5_1,
                            result5_2,
                            result5_3,
                            result5_4,
                            result5_5,
                            result5_6,
                            result5_7,
                            result5_8,
                            result5_9,
                            result5_10,
                            result5_11,
                        ) = match local_address {
                            V4::Ipv4(e) => {
                                let super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                    port: port0,
                                    address: address0,
                                } = e;
                                let (t1_0, t1_1, t1_2, t1_3) = address0;
                                (
                                    0i32,
                                    _rt::as_i32(port0),
                                    _rt::as_i32(t1_0),
                                    _rt::as_i32(t1_1),
                                    _rt::as_i32(t1_2),
                                    _rt::as_i32(t1_3),
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                )
                            }
                            V4::Ipv6(e) => {
                                let super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                    port: port2,
                                    flow_info: flow_info2,
                                    address: address2,
                                    scope_id: scope_id2,
                                } = e;
                                let (t3_0, t3_1, t3_2, t3_3, t3_4, t3_5, t3_6, t3_7) = address2;
                                (
                                    1i32,
                                    _rt::as_i32(port2),
                                    _rt::as_i32(flow_info2),
                                    _rt::as_i32(t3_0),
                                    _rt::as_i32(t3_1),
                                    _rt::as_i32(t3_2),
                                    _rt::as_i32(t3_3),
                                    _rt::as_i32(t3_4),
                                    _rt::as_i32(t3_5),
                                    _rt::as_i32(t3_6),
                                    _rt::as_i32(t3_7),
                                    _rt::as_i32(scope_id2),
                                )
                            }
                        };
                        let ptr6 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.start-bind"]
                            fn wit_import7(
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import7(
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import7(
                            (self).handle() as i32,
                            (network).handle() as i32,
                            result5_0,
                            result5_1,
                            result5_2,
                            result5_3,
                            result5_4,
                            result5_5,
                            result5_6,
                            result5_7,
                            result5_8,
                            result5_9,
                            result5_10,
                            result5_11,
                            ptr6,
                        );
                        let l8 = i32::from(*ptr6.add(0).cast::<u8>());
                        let result10 = match l8 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l9 = i32::from(*ptr6.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l9 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result10
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn finish_bind(&self) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.finish-bind"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn start_connect(
                    &self,
                    network: &Network,
                    remote_address: IpSocketAddress,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        use super::super::super::wasi::sockets::network::IpSocketAddress as V4;
                        let (
                            result5_0,
                            result5_1,
                            result5_2,
                            result5_3,
                            result5_4,
                            result5_5,
                            result5_6,
                            result5_7,
                            result5_8,
                            result5_9,
                            result5_10,
                            result5_11,
                        ) = match remote_address {
                            V4::Ipv4(e) => {
                                let super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                    port: port0,
                                    address: address0,
                                } = e;
                                let (t1_0, t1_1, t1_2, t1_3) = address0;
                                (
                                    0i32,
                                    _rt::as_i32(port0),
                                    _rt::as_i32(t1_0),
                                    _rt::as_i32(t1_1),
                                    _rt::as_i32(t1_2),
                                    _rt::as_i32(t1_3),
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                    0i32,
                                )
                            }
                            V4::Ipv6(e) => {
                                let super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                    port: port2,
                                    flow_info: flow_info2,
                                    address: address2,
                                    scope_id: scope_id2,
                                } = e;
                                let (t3_0, t3_1, t3_2, t3_3, t3_4, t3_5, t3_6, t3_7) = address2;
                                (
                                    1i32,
                                    _rt::as_i32(port2),
                                    _rt::as_i32(flow_info2),
                                    _rt::as_i32(t3_0),
                                    _rt::as_i32(t3_1),
                                    _rt::as_i32(t3_2),
                                    _rt::as_i32(t3_3),
                                    _rt::as_i32(t3_4),
                                    _rt::as_i32(t3_5),
                                    _rt::as_i32(t3_6),
                                    _rt::as_i32(t3_7),
                                    _rt::as_i32(scope_id2),
                                )
                            }
                        };
                        let ptr6 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.start-connect"]
                            fn wit_import7(
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: i32,
                                _: *mut u8,
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import7(
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: i32,
                            _: *mut u8,
                        ) {
                            unreachable!()
                        }
                        wit_import7(
                            (self).handle() as i32,
                            (network).handle() as i32,
                            result5_0,
                            result5_1,
                            result5_2,
                            result5_3,
                            result5_4,
                            result5_5,
                            result5_6,
                            result5_7,
                            result5_8,
                            result5_9,
                            result5_10,
                            result5_11,
                            ptr6,
                        );
                        let l8 = i32::from(*ptr6.add(0).cast::<u8>());
                        let result10 = match l8 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l9 = i32::from(*ptr6.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l9 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result10
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn finish_connect(
                    &self,
                ) -> Result<(InputStream, OutputStream), ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 12]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 12],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.finish-connect"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result6 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(4).cast::<i32>();
                                    let l4 = *ptr0.add(8).cast::<i32>();
                                    (
                                        super::super::super::wasi::io::streams::InputStream::from_handle(
                                            l3 as u32,
                                        ),
                                        super::super::super::wasi::io::streams::OutputStream::from_handle(
                                            l4 as u32,
                                        ),
                                    )
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l5 = i32::from(*ptr0.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l5 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result6
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn start_listen(&self) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.start-listen"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn finish_listen(&self) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.finish-listen"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn accept(
                    &self,
                ) -> Result<(TcpSocket, InputStream, OutputStream), ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.accept"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result7 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(4).cast::<i32>();
                                    let l4 = *ptr0.add(8).cast::<i32>();
                                    let l5 = *ptr0.add(12).cast::<i32>();
                                    (
                                        TcpSocket::from_handle(l3 as u32),
                                        super::super::super::wasi::io::streams::InputStream::from_handle(
                                            l4 as u32,
                                        ),
                                        super::super::super::wasi::io::streams::OutputStream::from_handle(
                                            l5 as u32,
                                        ),
                                    )
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l6 = i32::from(*ptr0.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l6 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result7
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn local_address(&self) -> Result<IpSocketAddress, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 36]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 36],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.local-address"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result22 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    use super::super::super::wasi::sockets::network::IpSocketAddress as V20;
                                    let v20 = match l3 {
                                        0 => {
                                            let e20 = {
                                                let l4 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l5 = i32::from(*ptr0.add(10).cast::<u8>());
                                                let l6 = i32::from(*ptr0.add(11).cast::<u8>());
                                                let l7 = i32::from(*ptr0.add(12).cast::<u8>());
                                                let l8 = i32::from(*ptr0.add(13).cast::<u8>());
                                                super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                                    port: l4 as u16,
                                                    address: (l5 as u8, l6 as u8, l7 as u8, l8 as u8),
                                                }
                                            };
                                            V20::Ipv4(e20)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            let e20 = {
                                                let l9 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l10 = *ptr0.add(12).cast::<i32>();
                                                let l11 = i32::from(*ptr0.add(16).cast::<u16>());
                                                let l12 = i32::from(*ptr0.add(18).cast::<u16>());
                                                let l13 = i32::from(*ptr0.add(20).cast::<u16>());
                                                let l14 = i32::from(*ptr0.add(22).cast::<u16>());
                                                let l15 = i32::from(*ptr0.add(24).cast::<u16>());
                                                let l16 = i32::from(*ptr0.add(26).cast::<u16>());
                                                let l17 = i32::from(*ptr0.add(28).cast::<u16>());
                                                let l18 = i32::from(*ptr0.add(30).cast::<u16>());
                                                let l19 = *ptr0.add(32).cast::<i32>();
                                                super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                                    port: l9 as u16,
                                                    flow_info: l10 as u32,
                                                    address: (
                                                        l11 as u16,
                                                        l12 as u16,
                                                        l13 as u16,
                                                        l14 as u16,
                                                        l15 as u16,
                                                        l16 as u16,
                                                        l17 as u16,
                                                        l18 as u16,
                                                    ),
                                                    scope_id: l19 as u32,
                                                }
                                            };
                                            V20::Ipv6(e20)
                                        }
                                    };
                                    v20
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l21 = i32::from(*ptr0.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l21 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result22
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn remote_address(&self) -> Result<IpSocketAddress, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 36]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 36],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.remote-address"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result22 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(4).cast::<u8>());
                                    use super::super::super::wasi::sockets::network::IpSocketAddress as V20;
                                    let v20 = match l3 {
                                        0 => {
                                            let e20 = {
                                                let l4 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l5 = i32::from(*ptr0.add(10).cast::<u8>());
                                                let l6 = i32::from(*ptr0.add(11).cast::<u8>());
                                                let l7 = i32::from(*ptr0.add(12).cast::<u8>());
                                                let l8 = i32::from(*ptr0.add(13).cast::<u8>());
                                                super::super::super::wasi::sockets::network::Ipv4SocketAddress {
                                                    port: l4 as u16,
                                                    address: (l5 as u8, l6 as u8, l7 as u8, l8 as u8),
                                                }
                                            };
                                            V20::Ipv4(e20)
                                        }
                                        n => {
                                            debug_assert_eq!(n, 1, "invalid enum discriminant");
                                            let e20 = {
                                                let l9 = i32::from(*ptr0.add(8).cast::<u16>());
                                                let l10 = *ptr0.add(12).cast::<i32>();
                                                let l11 = i32::from(*ptr0.add(16).cast::<u16>());
                                                let l12 = i32::from(*ptr0.add(18).cast::<u16>());
                                                let l13 = i32::from(*ptr0.add(20).cast::<u16>());
                                                let l14 = i32::from(*ptr0.add(22).cast::<u16>());
                                                let l15 = i32::from(*ptr0.add(24).cast::<u16>());
                                                let l16 = i32::from(*ptr0.add(26).cast::<u16>());
                                                let l17 = i32::from(*ptr0.add(28).cast::<u16>());
                                                let l18 = i32::from(*ptr0.add(30).cast::<u16>());
                                                let l19 = *ptr0.add(32).cast::<i32>();
                                                super::super::super::wasi::sockets::network::Ipv6SocketAddress {
                                                    port: l9 as u16,
                                                    flow_info: l10 as u32,
                                                    address: (
                                                        l11 as u16,
                                                        l12 as u16,
                                                        l13 as u16,
                                                        l14 as u16,
                                                        l15 as u16,
                                                        l16 as u16,
                                                        l17 as u16,
                                                        l18 as u16,
                                                    ),
                                                    scope_id: l19 as u32,
                                                }
                                            };
                                            V20::Ipv6(e20)
                                        }
                                    };
                                    v20
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l21 = i32::from(*ptr0.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l21 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result22
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn is_listening(&self) -> bool {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.is-listening"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        _rt::bool_lift(ret as u8)
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn address_family(&self) -> IpAddressFamily {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.address-family"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::sockets::network::IpAddressFamily::_lift(
                            ret as u8,
                        )
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_listen_backlog_size(
                    &self,
                    value: u64,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-listen-backlog-size"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn keep_alive_enabled(&self) -> Result<bool, ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.keep-alive-enabled"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    _rt::bool_lift(l3 as u8)
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_keep_alive_enabled(
                    &self,
                    value: bool,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-keep-alive-enabled"]
                            fn wit_import1(_: i32, _: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1(
                            (self).handle() as i32,
                            match &value {
                                true => 1,
                                false => 0,
                            },
                            ptr0,
                        );
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn keep_alive_idle_time(&self) -> Result<Duration, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.keep-alive-idle-time"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_keep_alive_idle_time(
                    &self,
                    value: Duration,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-keep-alive-idle-time"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn keep_alive_interval(&self) -> Result<Duration, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.keep-alive-interval"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_keep_alive_interval(
                    &self,
                    value: Duration,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-keep-alive-interval"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn keep_alive_count(&self) -> Result<u32, ErrorCode> {
                    unsafe {
                        #[repr(align(4))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 8],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.keep-alive-count"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(4).cast::<i32>();
                                    l3 as u32
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_keep_alive_count(&self, value: u32) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-keep-alive-count"]
                            fn wit_import1(_: i32, _: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i32(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn hop_limit(&self) -> Result<u8, ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.hop-limit"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    l3 as u8
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_hop_limit(&self, value: u8) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-hop-limit"]
                            fn wit_import1(_: i32, _: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i32(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn receive_buffer_size(&self) -> Result<u64, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.receive-buffer-size"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_receive_buffer_size(
                    &self,
                    value: u64,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-receive-buffer-size"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn send_buffer_size(&self) -> Result<u64, ErrorCode> {
                    unsafe {
                        #[repr(align(8))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 16]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 16],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.send-buffer-size"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result5 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = *ptr0.add(8).cast::<i64>();
                                    l3 as u64
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l4 = i32::from(*ptr0.add(8).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l4 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result5
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn set_send_buffer_size(&self, value: u64) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.set-send-buffer-size"]
                            fn wit_import1(_: i32, _: i64, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, _rt::as_i64(&value), ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn subscribe(&self) -> Pollable {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.subscribe"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::io::poll::Pollable::from_handle(
                            ret as u32,
                        )
                    }
                }
            }
            impl TcpSocket {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn shutdown(
                    &self,
                    shutdown_type: ShutdownType,
                ) -> Result<(), ErrorCode> {
                    unsafe {
                        #[repr(align(1))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 2]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 2],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/tcp@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]tcp-socket.shutdown"]
                            fn wit_import1(_: i32, _: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1(
                            (self).handle() as i32,
                            shutdown_type.clone() as i32,
                            ptr0,
                        );
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result4 = match l2 {
                            0 => {
                                let e = ();
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(1).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l3 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result4
                    }
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod tcp_create_socket {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Network = super::super::super::wasi::sockets::network::Network;
            pub type ErrorCode = super::super::super::wasi::sockets::network::ErrorCode;
            pub type IpAddressFamily = super::super::super::wasi::sockets::network::IpAddressFamily;
            pub type TcpSocket = super::super::super::wasi::sockets::tcp::TcpSocket;
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn create_tcp_socket(
                address_family: IpAddressFamily,
            ) -> Result<TcpSocket, ErrorCode> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/tcp-create-socket@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "create-tcp-socket"]
                        fn wit_import1(_: i32, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                        unreachable!()
                    }
                    wit_import1(address_family.clone() as i32, ptr0);
                    let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                    let result5 = match l2 {
                        0 => {
                            let e = {
                                let l3 = *ptr0.add(4).cast::<i32>();
                                super::super::super::wasi::sockets::tcp::TcpSocket::from_handle(
                                    l3 as u32,
                                )
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                    l4 as u8,
                                )
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result5
                }
            }
        }
        #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
        pub mod ip_name_lookup {
            #[used]
            #[doc(hidden)]
            static __FORCE_SECTION_REF: fn() = super::super::super::__link_custom_section_describing_imports;
            use super::super::super::_rt;
            pub type Pollable = super::super::super::wasi::io::poll::Pollable;
            pub type Network = super::super::super::wasi::sockets::network::Network;
            pub type ErrorCode = super::super::super::wasi::sockets::network::ErrorCode;
            pub type IpAddress = super::super::super::wasi::sockets::network::IpAddress;
            #[derive(Debug)]
            #[repr(transparent)]
            pub struct ResolveAddressStream {
                handle: _rt::Resource<ResolveAddressStream>,
            }
            impl ResolveAddressStream {
                #[doc(hidden)]
                pub unsafe fn from_handle(handle: u32) -> Self {
                    Self {
                        handle: unsafe { _rt::Resource::from_handle(handle) },
                    }
                }
                #[doc(hidden)]
                pub fn take_handle(&self) -> u32 {
                    _rt::Resource::take_handle(&self.handle)
                }
                #[doc(hidden)]
                pub fn handle(&self) -> u32 {
                    _rt::Resource::handle(&self.handle)
                }
            }
            unsafe impl _rt::WasmResource for ResolveAddressStream {
                #[inline]
                unsafe fn drop(_handle: u32) {
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/ip-name-lookup@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "[resource-drop]resolve-address-stream"]
                        fn drop(_: i32);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn drop(_: i32) {
                        unreachable!()
                    }
                    unsafe {
                        drop(_handle as i32);
                    }
                }
            }
            impl ResolveAddressStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn resolve_next_address(
                    &self,
                ) -> Result<Option<IpAddress>, ErrorCode> {
                    unsafe {
                        #[repr(align(2))]
                        struct RetArea([::core::mem::MaybeUninit<u8>; 22]);
                        let mut ret_area = RetArea(
                            [::core::mem::MaybeUninit::uninit(); 22],
                        );
                        let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/ip-name-lookup@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]resolve-address-stream.resolve-next-address"]
                            fn wit_import1(_: i32, _: *mut u8);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import1(_: i32, _: *mut u8) {
                            unreachable!()
                        }
                        wit_import1((self).handle() as i32, ptr0);
                        let l2 = i32::from(*ptr0.add(0).cast::<u8>());
                        let result19 = match l2 {
                            0 => {
                                let e = {
                                    let l3 = i32::from(*ptr0.add(2).cast::<u8>());
                                    match l3 {
                                        0 => None,
                                        1 => {
                                            let e = {
                                                let l4 = i32::from(*ptr0.add(4).cast::<u8>());
                                                use super::super::super::wasi::sockets::network::IpAddress as V17;
                                                let v17 = match l4 {
                                                    0 => {
                                                        let e17 = {
                                                            let l5 = i32::from(*ptr0.add(6).cast::<u8>());
                                                            let l6 = i32::from(*ptr0.add(7).cast::<u8>());
                                                            let l7 = i32::from(*ptr0.add(8).cast::<u8>());
                                                            let l8 = i32::from(*ptr0.add(9).cast::<u8>());
                                                            (l5 as u8, l6 as u8, l7 as u8, l8 as u8)
                                                        };
                                                        V17::Ipv4(e17)
                                                    }
                                                    n => {
                                                        debug_assert_eq!(n, 1, "invalid enum discriminant");
                                                        let e17 = {
                                                            let l9 = i32::from(*ptr0.add(6).cast::<u16>());
                                                            let l10 = i32::from(*ptr0.add(8).cast::<u16>());
                                                            let l11 = i32::from(*ptr0.add(10).cast::<u16>());
                                                            let l12 = i32::from(*ptr0.add(12).cast::<u16>());
                                                            let l13 = i32::from(*ptr0.add(14).cast::<u16>());
                                                            let l14 = i32::from(*ptr0.add(16).cast::<u16>());
                                                            let l15 = i32::from(*ptr0.add(18).cast::<u16>());
                                                            let l16 = i32::from(*ptr0.add(20).cast::<u16>());
                                                            (
                                                                l9 as u16,
                                                                l10 as u16,
                                                                l11 as u16,
                                                                l12 as u16,
                                                                l13 as u16,
                                                                l14 as u16,
                                                                l15 as u16,
                                                                l16 as u16,
                                                            )
                                                        };
                                                        V17::Ipv6(e17)
                                                    }
                                                };
                                                v17
                                            };
                                            Some(e)
                                        }
                                        _ => _rt::invalid_enum_discriminant(),
                                    }
                                };
                                Ok(e)
                            }
                            1 => {
                                let e = {
                                    let l18 = i32::from(*ptr0.add(2).cast::<u8>());
                                    super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                        l18 as u8,
                                    )
                                };
                                Err(e)
                            }
                            _ => _rt::invalid_enum_discriminant(),
                        };
                        result19
                    }
                }
            }
            impl ResolveAddressStream {
                #[allow(unused_unsafe, clippy::all)]
                #[allow(async_fn_in_trait)]
                pub fn subscribe(&self) -> Pollable {
                    unsafe {
                        #[cfg(target_arch = "wasm32")]
                        #[link(wasm_import_module = "wasi:sockets/ip-name-lookup@0.2.6")]
                        unsafe extern "C" {
                            #[link_name = "[method]resolve-address-stream.subscribe"]
                            fn wit_import0(_: i32) -> i32;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        unsafe extern "C" fn wit_import0(_: i32) -> i32 {
                            unreachable!()
                        }
                        let ret = wit_import0((self).handle() as i32);
                        super::super::super::wasi::io::poll::Pollable::from_handle(
                            ret as u32,
                        )
                    }
                }
            }
            #[allow(unused_unsafe, clippy::all)]
            #[allow(async_fn_in_trait)]
            pub fn resolve_addresses(
                network: &Network,
                name: &[u8],
            ) -> Result<ResolveAddressStream, ErrorCode> {
                unsafe {
                    #[repr(align(4))]
                    struct RetArea([::core::mem::MaybeUninit<u8>; 8]);
                    let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
                    let vec0 = name;
                    let ptr0 = vec0.as_ptr().cast::<u8>();
                    let len0 = vec0.len();
                    let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
                    #[cfg(target_arch = "wasm32")]
                    #[link(wasm_import_module = "wasi:sockets/ip-name-lookup@0.2.6")]
                    unsafe extern "C" {
                        #[link_name = "resolve-addresses"]
                        fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    unsafe extern "C" fn wit_import2(
                        _: i32,
                        _: *mut u8,
                        _: usize,
                        _: *mut u8,
                    ) {
                        unreachable!()
                    }
                    wit_import2((network).handle() as i32, ptr0.cast_mut(), len0, ptr1);
                    let l3 = i32::from(*ptr1.add(0).cast::<u8>());
                    let result6 = match l3 {
                        0 => {
                            let e = {
                                let l4 = *ptr1.add(4).cast::<i32>();
                                ResolveAddressStream::from_handle(l4 as u32)
                            };
                            Ok(e)
                        }
                        1 => {
                            let e = {
                                let l5 = i32::from(*ptr1.add(4).cast::<u8>());
                                super::super::super::wasi::sockets::network::ErrorCode::_lift(
                                    l5 as u8,
                                )
                            };
                            Err(e)
                        }
                        _ => _rt::invalid_enum_discriminant(),
                    };
                    result6
                }
            }
        }
    }
}
#[rustfmt::skip]
#[allow(dead_code, clippy::all)]
pub mod exports {
    pub mod catscope {
        pub mod witbot {
            #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
            pub mod updater {
                #[used]
                #[doc(hidden)]
                static __FORCE_SECTION_REF: fn() = super::super::super::super::__link_custom_section_describing_imports;
                use super::super::super::super::_rt;
                #[doc(hidden)]
                #[allow(non_snake_case, unused_unsafe)]
                pub unsafe fn _export_transactionv1_cabi<T: Guest>(
                    arg0: *mut u8,
                    arg1: usize,
                    arg2: i64,
                    arg3: i32,
                    arg4: i32,
                ) {
                    unsafe {
                        #[cfg(target_arch = "wasm32")] _rt::run_ctors_once();
                        {
                            let len0 = arg1;
                            T::transactionv1(
                                _rt::Vec::from_raw_parts(arg0.cast(), len0, len0),
                                arg2 as u64,
                                arg3 as u8,
                                arg4 as u32,
                            )
                        };
                    }
                }
                #[doc(hidden)]
                #[allow(non_snake_case, unused_unsafe)]
                pub unsafe fn _export_accountv1_cabi<T: Guest>(
                    arg0: i64,
                    arg1: i64,
                    arg2: i64,
                    arg3: i64,
                    arg4: i64,
                    arg5: i32,
                ) {
                    unsafe {
                        #[cfg(target_arch = "wasm32")] _rt::run_ctors_once();
                        {
                            T::accountv1(
                                arg0 as u64,
                                arg1 as u64,
                                arg2 as u64,
                                arg3 as u64,
                                arg4 as u64,
                                arg5 as u32,
                            )
                        };
                    }
                }
                #[doc(hidden)]
                #[allow(non_snake_case, unused_unsafe)]
                pub unsafe fn _export_tokenv1_cabi<T: Guest>(
                    arg0: i64,
                    arg1: i64,
                    arg2: i64,
                    arg3: i64,
                    arg4: i64,
                    arg5: i64,
                ) {
                    unsafe {
                        #[cfg(target_arch = "wasm32")] _rt::run_ctors_once();
                        {
                            T::tokenv1(
                                arg0 as u64,
                                arg1 as u64,
                                arg2 as u64,
                                arg3 as u64,
                                arg4 as u64,
                                arg5 as u64,
                            )
                        };
                    }
                }
                #[doc(hidden)]
                #[allow(non_snake_case, unused_unsafe)]
                pub unsafe fn _export_slot_cabi<T: Guest>(arg0: i64, arg1: i32) {
                    unsafe {
                        #[cfg(target_arch = "wasm32")] _rt::run_ctors_once();
                        { T::slot(arg0 as u64, arg1 as u8) };
                    }
                }
                pub trait Guest {
                    #[allow(async_fn_in_trait)]
                    fn transactionv1(
                        signature: _rt::Vec<u8>,
                        slot: u64,
                        status: u8,
                        txresult: u32,
                    ) -> ();
                    #[allow(async_fn_in_trait)]
                    fn accountv1(
                        id: u64,
                        slot: u64,
                        lamports: u64,
                        rent: u64,
                        owner: u64,
                        datasize: u32,
                    ) -> ();
                    #[allow(async_fn_in_trait)]
                    fn tokenv1(
                        id: u64,
                        slot: u64,
                        lamports: u64,
                        mint: u64,
                        owner: u64,
                        balance: u64,
                    ) -> ();
                    #[allow(async_fn_in_trait)]
                    fn slot(slot: u64, status: u8) -> ();
                }
                #[doc(hidden)]
                #[macro_export]
                macro_rules! __export_catscope_witbot_updater_0_1_0_cabi {
                    ($ty:ident with_types_in $($path_to_types:tt)*) => {
                        const _ : () = { #[unsafe (export_name =
                        "catscope:witbot/updater@0.1.0#transactionv1")] unsafe extern "C"
                        fn export_transactionv1(arg0 : * mut u8, arg1 : usize, arg2 :
                        i64, arg3 : i32, arg4 : i32,) { unsafe { $($path_to_types)*::
                        _export_transactionv1_cabi::<$ty > (arg0, arg1, arg2, arg3, arg4)
                        } } #[unsafe (export_name =
                        "catscope:witbot/updater@0.1.0#accountv1")] unsafe extern "C" fn
                        export_accountv1(arg0 : i64, arg1 : i64, arg2 : i64, arg3 : i64,
                        arg4 : i64, arg5 : i32,) { unsafe { $($path_to_types)*::
                        _export_accountv1_cabi::<$ty > (arg0, arg1, arg2, arg3, arg4,
                        arg5) } } #[unsafe (export_name =
                        "catscope:witbot/updater@0.1.0#tokenv1")] unsafe extern "C" fn
                        export_tokenv1(arg0 : i64, arg1 : i64, arg2 : i64, arg3 : i64,
                        arg4 : i64, arg5 : i64,) { unsafe { $($path_to_types)*::
                        _export_tokenv1_cabi::<$ty > (arg0, arg1, arg2, arg3, arg4, arg5)
                        } } #[unsafe (export_name =
                        "catscope:witbot/updater@0.1.0#slot")] unsafe extern "C" fn
                        export_slot(arg0 : i64, arg1 : i32,) { unsafe {
                        $($path_to_types)*:: _export_slot_cabi::<$ty > (arg0, arg1) } }
                        };
                    };
                }
                #[doc(hidden)]
                pub use __export_catscope_witbot_updater_0_1_0_cabi;
            }
        }
    }
    pub mod wasi {
        pub mod cli {
            #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
            pub mod run {
                #[used]
                #[doc(hidden)]
                static __FORCE_SECTION_REF: fn() = super::super::super::super::__link_custom_section_describing_imports;
                use super::super::super::super::_rt;
                #[doc(hidden)]
                #[allow(non_snake_case, unused_unsafe)]
                pub unsafe fn _export_run_cabi<T: Guest>() -> i32 {
                    unsafe {
                        #[cfg(target_arch = "wasm32")] _rt::run_ctors_once();
                        let result0 = { T::run() };
                        let result1 = match result0 {
                            Ok(_) => 0i32,
                            Err(_) => 1i32,
                        };
                        result1
                    }
                }
                pub trait Guest {
                    /// Run the program.
                    #[allow(async_fn_in_trait)]
                    fn run() -> Result<(), ()>;
                }
                #[doc(hidden)]
                #[macro_export]
                macro_rules! __export_wasi_cli_run_0_2_6_cabi {
                    ($ty:ident with_types_in $($path_to_types:tt)*) => {
                        const _ : () = { #[unsafe (export_name =
                        "wasi:cli/run@0.2.6#run")] unsafe extern "C" fn export_run() ->
                        i32 { unsafe { $($path_to_types)*:: _export_run_cabi::<$ty > () }
                        } };
                    };
                }
                #[doc(hidden)]
                pub use __export_wasi_cli_run_0_2_6_cabi;
            }
        }
    }
}
#[rustfmt::skip]
mod _rt {
    #![allow(dead_code, clippy::all)]
    pub unsafe fn invalid_enum_discriminant<T>() -> T {
        if cfg!(debug_assertions) {
            panic!("invalid enum discriminant")
        } else {
            unsafe { core::hint::unreachable_unchecked() }
        }
    }
    pub fn as_i64<T: AsI64>(t: T) -> i64 {
        t.as_i64()
    }
    pub trait AsI64 {
        fn as_i64(self) -> i64;
    }
    impl<'a, T: Copy + AsI64> AsI64 for &'a T {
        fn as_i64(self) -> i64 {
            (*self).as_i64()
        }
    }
    impl AsI64 for i64 {
        #[inline]
        fn as_i64(self) -> i64 {
            self as i64
        }
    }
    impl AsI64 for u64 {
        #[inline]
        fn as_i64(self) -> i64 {
            self as i64
        }
    }
    pub use alloc_crate::vec::Vec;
    pub fn as_i32<T: AsI32>(t: T) -> i32 {
        t.as_i32()
    }
    pub trait AsI32 {
        fn as_i32(self) -> i32;
    }
    impl<'a, T: Copy + AsI32> AsI32 for &'a T {
        fn as_i32(self) -> i32 {
            (*self).as_i32()
        }
    }
    impl AsI32 for i32 {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    impl AsI32 for u32 {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    impl AsI32 for i16 {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    impl AsI32 for u16 {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    impl AsI32 for i8 {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    impl AsI32 for u8 {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    impl AsI32 for char {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    impl AsI32 for usize {
        #[inline]
        fn as_i32(self) -> i32 {
            self as i32
        }
    }
    use core::fmt;
    use core::marker;
    use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
    /// A type which represents a component model resource, either imported or
    /// exported into this component.
    ///
    /// This is a low-level wrapper which handles the lifetime of the resource
    /// (namely this has a destructor). The `T` provided defines the component model
    /// intrinsics that this wrapper uses.
    ///
    /// One of the chief purposes of this type is to provide `Deref` implementations
    /// to access the underlying data when it is owned.
    ///
    /// This type is primarily used in generated code for exported and imported
    /// resources.
    #[repr(transparent)]
    pub struct Resource<T: WasmResource> {
        handle: AtomicU32,
        _marker: marker::PhantomData<T>,
    }
    /// A trait which all wasm resources implement, namely providing the ability to
    /// drop a resource.
    ///
    /// This generally is implemented by generated code, not user-facing code.
    #[allow(clippy::missing_safety_doc)]
    pub unsafe trait WasmResource {
        /// Invokes the `[resource-drop]...` intrinsic.
        unsafe fn drop(handle: u32);
    }
    impl<T: WasmResource> Resource<T> {
        #[doc(hidden)]
        pub unsafe fn from_handle(handle: u32) -> Self {
            debug_assert!(handle != 0 && handle != u32::MAX);
            Self {
                handle: AtomicU32::new(handle),
                _marker: marker::PhantomData,
            }
        }
        /// Takes ownership of the handle owned by `resource`.
        ///
        /// Note that this ideally would be `into_handle` taking `Resource<T>` by
        /// ownership. The code generator does not enable that in all situations,
        /// unfortunately, so this is provided instead.
        ///
        /// Also note that `take_handle` is in theory only ever called on values
        /// owned by a generated function. For example a generated function might
        /// take `Resource<T>` as an argument but then call `take_handle` on a
        /// reference to that argument. In that sense the dynamic nature of
        /// `take_handle` should only be exposed internally to generated code, not
        /// to user code.
        #[doc(hidden)]
        pub fn take_handle(resource: &Resource<T>) -> u32 {
            resource.handle.swap(u32::MAX, Relaxed)
        }
        #[doc(hidden)]
        pub fn handle(resource: &Resource<T>) -> u32 {
            resource.handle.load(Relaxed)
        }
    }
    impl<T: WasmResource> fmt::Debug for Resource<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Resource").field("handle", &self.handle).finish()
        }
    }
    impl<T: WasmResource> Drop for Resource<T> {
        fn drop(&mut self) {
            unsafe {
                match self.handle.load(Relaxed) {
                    u32::MAX => {}
                    other => T::drop(other),
                }
            }
        }
    }
    pub unsafe fn bool_lift(val: u8) -> bool {
        if cfg!(debug_assertions) {
            match val {
                0 => false,
                1 => true,
                _ => panic!("invalid bool discriminant"),
            }
        } else {
            val != 0
        }
    }
    pub use alloc_crate::alloc;
    pub unsafe fn cabi_dealloc(ptr: *mut u8, size: usize, align: usize) {
        if size == 0 {
            return;
        }
        unsafe {
            let layout = alloc::Layout::from_size_align_unchecked(size, align);
            alloc::dealloc(ptr, layout);
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn run_ctors_once() {
        wit_bindgen::rt::run_ctors_once();
    }
    extern crate alloc as alloc_crate;
}
pub mod wit_future {
    #![allow(dead_code, unused_variables, clippy::all)]
    #[doc(hidden)]
    pub trait FuturePayload: Unpin + Sized + 'static {
        const VTABLE: &'static wit_bindgen::rt::async_support::FutureVtable<Self>;
    }
    #[doc(hidden)]
    #[allow(unused_unsafe)]
    pub mod vtable0 {
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn cancel_write(_: u32) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn cancel_read(_: u32) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn drop_writable(_: u32) {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn drop_readable(_: u32) {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn new() -> u64 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn start_read(_: u32, _: *mut u8) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn start_write(_: u32, _: *const u8) -> u32 {
            unreachable!()
        }
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "catscope:witbot/transactionprocessor@0.1.0")]
        unsafe extern "C" {
            #[link_name = "[future-new-0]send"]
            fn new() -> u64;
            #[link_name = "[future-cancel-write-0]send"]
            fn cancel_write(_: u32) -> u32;
            #[link_name = "[future-cancel-read-0]send"]
            fn cancel_read(_: u32) -> u32;
            #[link_name = "[future-drop-writable-0]send"]
            fn drop_writable(_: u32);
            #[link_name = "[future-drop-readable-0]send"]
            fn drop_readable(_: u32);
            #[link_name = "[async-lower][future-read-0]send"]
            fn start_read(_: u32, _: *mut u8) -> u32;
            #[link_name = "[async-lower][future-write-0]send"]
            fn start_write(_: u32, _: *const u8) -> u32;
        }
        unsafe fn lift(
            ptr: *mut u8,
        ) -> Result<
            u64,
            super::super::catscope::witbot::transactionprocessor::ErrorCode,
        > {
            unsafe {
                let l0 = i32::from(*ptr.add(0).cast::<u8>());
                match l0 {
                    0 => {
                        let e = {
                            let l1 = *ptr.add(8).cast::<i64>();
                            l1 as u64
                        };
                        Ok(e)
                    }
                    1 => {
                        let e = {
                            let l2 = i32::from(*ptr.add(8).cast::<u8>());
                            super::super::catscope::witbot::transactionprocessor::ErrorCode::_lift(
                                l2 as u8,
                            )
                        };
                        Err(e)
                    }
                    _ => super::super::_rt::invalid_enum_discriminant(),
                }
            }
        }
        unsafe fn lower(
            value: Result<
                u64,
                super::super::catscope::witbot::transactionprocessor::ErrorCode,
            >,
            ptr: *mut u8,
        ) {
            unsafe {
                match value {
                    Ok(e) => {
                        *ptr.add(0).cast::<u8>() = (0i32) as u8;
                        *ptr.add(8).cast::<i64>() = super::super::_rt::as_i64(e);
                    }
                    Err(e) => {
                        *ptr.add(0).cast::<u8>() = (1i32) as u8;
                        *ptr.add(8).cast::<u8>() = (e.clone() as i32) as u8;
                    }
                };
            }
        }
        unsafe fn dealloc_lists(ptr: *mut u8) {
            unsafe {}
        }
        pub static VTABLE: wit_bindgen::rt::async_support::FutureVtable<
            Result<u64, super::super::catscope::witbot::transactionprocessor::ErrorCode>,
        > = wit_bindgen::rt::async_support::FutureVtable::<
            Result<u64, super::super::catscope::witbot::transactionprocessor::ErrorCode>,
        > {
            cancel_write,
            cancel_read,
            drop_writable,
            drop_readable,
            dealloc_lists,
            layout: unsafe { ::std::alloc::Layout::from_size_align_unchecked(16, 8) },
            lift,
            lower,
            new,
            start_read,
            start_write,
        };
        impl super::FuturePayload
        for Result<
            u64,
            super::super::catscope::witbot::transactionprocessor::ErrorCode,
        > {
            const VTABLE: &'static wit_bindgen::rt::async_support::FutureVtable<Self> = &VTABLE;
        }
    }
    #[doc(hidden)]
    #[allow(unused_unsafe)]
    pub mod vtable1 {
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn cancel_write(_: u32) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn cancel_read(_: u32) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn drop_writable(_: u32) {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn drop_readable(_: u32) {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn new() -> u64 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn start_read(_: u32, _: *mut u8) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn start_write(_: u32, _: *const u8) -> u32 {
            unreachable!()
        }
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
        unsafe extern "C" {
            #[link_name = "[future-new-0][method]subscription.subscribe"]
            fn new() -> u64;
            #[link_name = "[future-cancel-write-0][method]subscription.subscribe"]
            fn cancel_write(_: u32) -> u32;
            #[link_name = "[future-cancel-read-0][method]subscription.subscribe"]
            fn cancel_read(_: u32) -> u32;
            #[link_name = "[future-drop-writable-0][method]subscription.subscribe"]
            fn drop_writable(_: u32);
            #[link_name = "[future-drop-readable-0][method]subscription.subscribe"]
            fn drop_readable(_: u32);
            #[link_name = "[async-lower][future-read-0][method]subscription.subscribe"]
            fn start_read(_: u32, _: *mut u8) -> u32;
            #[link_name = "[async-lower][future-write-0][method]subscription.subscribe"]
            fn start_write(_: u32, _: *const u8) -> u32;
        }
        unsafe fn lift(ptr: *mut u8) -> u32 {
            unsafe {
                let l0 = *ptr.add(0).cast::<i32>();
                l0 as u32
            }
        }
        unsafe fn lower(value: u32, ptr: *mut u8) {
            unsafe {
                *ptr.add(0).cast::<i32>() = super::super::_rt::as_i32(value);
            }
        }
        unsafe fn dealloc_lists(ptr: *mut u8) {
            unsafe {}
        }
        pub static VTABLE: wit_bindgen::rt::async_support::FutureVtable<u32> = wit_bindgen::rt::async_support::FutureVtable::<
            u32,
        > {
            cancel_write,
            cancel_read,
            drop_writable,
            drop_readable,
            dealloc_lists,
            layout: unsafe { ::std::alloc::Layout::from_size_align_unchecked(4, 4) },
            lift,
            lower,
            new,
            start_read,
            start_write,
        };
        impl super::FuturePayload for u32 {
            const VTABLE: &'static wit_bindgen::rt::async_support::FutureVtable<Self> = &VTABLE;
        }
    }
    #[doc(hidden)]
    #[allow(unused_unsafe)]
    pub mod vtable2 {
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn cancel_write(_: u32) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn cancel_read(_: u32) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn drop_writable(_: u32) {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn drop_readable(_: u32) {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn new() -> u64 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn start_read(_: u32, _: *mut u8) -> u32 {
            unreachable!()
        }
        #[cfg(not(target_arch = "wasm32"))]
        unsafe extern "C" fn start_write(_: u32, _: *const u8) -> u32 {
            unreachable!()
        }
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
        unsafe extern "C" {
            #[link_name = "[future-new-0]account-by-id"]
            fn new() -> u64;
            #[link_name = "[future-cancel-write-0]account-by-id"]
            fn cancel_write(_: u32) -> u32;
            #[link_name = "[future-cancel-read-0]account-by-id"]
            fn cancel_read(_: u32) -> u32;
            #[link_name = "[future-drop-writable-0]account-by-id"]
            fn drop_writable(_: u32);
            #[link_name = "[future-drop-readable-0]account-by-id"]
            fn drop_readable(_: u32);
            #[link_name = "[async-lower][future-read-0]account-by-id"]
            fn start_read(_: u32, _: *mut u8) -> u32;
            #[link_name = "[async-lower][future-write-0]account-by-id"]
            fn start_write(_: u32, _: *const u8) -> u32;
        }
        unsafe fn lift(
            ptr: *mut u8,
        ) -> Result<
            super::super::_rt::Vec<u8>,
            super::super::catscope::witbot::shooter::ErrorCode,
        > {
            unsafe {
                let l0 = i32::from(*ptr.add(0).cast::<u8>());
                match l0 {
                    0 => {
                        let e = {
                            let l1 = *ptr
                                .add(::core::mem::size_of::<*const u8>())
                                .cast::<*mut u8>();
                            let l2 = *ptr
                                .add(2 * ::core::mem::size_of::<*const u8>())
                                .cast::<usize>();
                            let len3 = l2;
                            super::super::_rt::Vec::from_raw_parts(l1.cast(), len3, len3)
                        };
                        Ok(e)
                    }
                    1 => {
                        let e = {
                            let l4 = i32::from(
                                *ptr.add(::core::mem::size_of::<*const u8>()).cast::<u8>(),
                            );
                            super::super::catscope::witbot::shooter::ErrorCode::_lift(
                                l4 as u8,
                            )
                        };
                        Err(e)
                    }
                    _ => super::super::_rt::invalid_enum_discriminant(),
                }
            }
        }
        unsafe fn lower(
            value: Result<
                super::super::_rt::Vec<u8>,
                super::super::catscope::witbot::shooter::ErrorCode,
            >,
            ptr: *mut u8,
        ) {
            unsafe {
                match value {
                    Ok(e) => {
                        *ptr.add(0).cast::<u8>() = (0i32) as u8;
                        let vec0 = (e).into_boxed_slice();
                        let ptr0 = vec0.as_ptr().cast::<u8>();
                        let len0 = vec0.len();
                        ::core::mem::forget(vec0);
                        *ptr
                            .add(2 * ::core::mem::size_of::<*const u8>())
                            .cast::<usize>() = len0;
                        *ptr
                            .add(::core::mem::size_of::<*const u8>())
                            .cast::<*mut u8>() = ptr0.cast_mut();
                    }
                    Err(e) => {
                        *ptr.add(0).cast::<u8>() = (1i32) as u8;
                        *ptr.add(::core::mem::size_of::<*const u8>()).cast::<u8>() = (e
                            .clone() as i32) as u8;
                    }
                };
            }
        }
        unsafe fn dealloc_lists(ptr: *mut u8) {
            unsafe {
                let l0 = i32::from(*ptr.add(0).cast::<u8>());
                match l0 {
                    0 => {
                        let l1 = *ptr
                            .add(::core::mem::size_of::<*const u8>())
                            .cast::<*mut u8>();
                        let l2 = *ptr
                            .add(2 * ::core::mem::size_of::<*const u8>())
                            .cast::<usize>();
                        let base3 = l1;
                        let len3 = l2;
                        super::super::_rt::cabi_dealloc(base3, len3 * 1, 1);
                    }
                    _ => {}
                }
            }
        }
        pub static VTABLE: wit_bindgen::rt::async_support::FutureVtable<
            Result<
                super::super::_rt::Vec<u8>,
                super::super::catscope::witbot::shooter::ErrorCode,
            >,
        > = wit_bindgen::rt::async_support::FutureVtable::<
            Result<
                super::super::_rt::Vec<u8>,
                super::super::catscope::witbot::shooter::ErrorCode,
            >,
        > {
            cancel_write,
            cancel_read,
            drop_writable,
            drop_readable,
            dealloc_lists,
            layout: unsafe { ::std::alloc::Layout::from_size_align_unchecked(12, 4) },
            lift,
            lower,
            new,
            start_read,
            start_write,
        };
        impl super::FuturePayload
        for Result<
            super::super::_rt::Vec<u8>,
            super::super::catscope::witbot::shooter::ErrorCode,
        > {
            const VTABLE: &'static wit_bindgen::rt::async_support::FutureVtable<Self> = &VTABLE;
        }
    }
    /// Creates a new Component Model `future` with the specified payload type.
    ///
    /// The `default` function provided computes the default value to be sent in
    /// this future if no other value was otherwise sent.
    pub fn new<T: FuturePayload>(
        default: fn() -> T,
    ) -> (
        wit_bindgen::rt::async_support::FutureWriter<T>,
        wit_bindgen::rt::async_support::FutureReader<T>,
    ) {
        unsafe { wit_bindgen::rt::async_support::future_new::<T>(default, T::VTABLE) }
    }
}
/// Generates `#[unsafe(no_mangle)]` functions to export the specified type as
/// the root implementation of all generated traits.
///
/// For more information see the documentation of `wit_bindgen::generate!`.
///
/// ```rust
/// # macro_rules! export{ ($($t:tt)*) => (); }
/// # trait Guest {}
/// struct MyType;
///
/// impl Guest for MyType {
///     // ...
/// }
///
/// export!(MyType);
/// ```
#[allow(unused_macros)]
#[doc(hidden)]
#[macro_export]
macro_rules! __export_catscopevalidator_impl {
    ($ty:ident) => {
        self::export!($ty with_types_in self);
    };
    ($ty:ident with_types_in $($path_to_types_root:tt)*) => {
        $($path_to_types_root)*::
        exports::catscope::witbot::updater::__export_catscope_witbot_updater_0_1_0_cabi!($ty
        with_types_in $($path_to_types_root)*:: exports::catscope::witbot::updater);
        $($path_to_types_root)*::
        exports::wasi::cli::run::__export_wasi_cli_run_0_2_6_cabi!($ty with_types_in
        $($path_to_types_root)*:: exports::wasi::cli::run); const _ : () = {
        #[rustfmt::skip] #[cfg(target_arch = "wasm32")] #[unsafe (link_section =
        "component-type:wit-bindgen:0.44.0:catscope:witbot@0.1.0:catscopevalidator:imports and exports")]
        #[doc(hidden)] #[allow(clippy::octal_escapes)] pub static
        __WIT_BINDGEN_COMPONENT_TYPE : [u8; 11902] = *
        b"\
\0asm\x0d\0\x01\0\0\x19\x16wit-component-encoding\x04\0\x07\xf6[\x01A\x02\x01AO\x01\
B\x0d\x01m\x06\x07unknown\x0daccess-denied\x0dnot-supported\x10invalid-argument\x0d\
out-of-memory\x07timeout\x04\0\x0aerror-code\x03\0\0\x01p}\x01j\x01w\x01\x01\x01\
e\x01\x03\x01@\x02\x09signature\x02\x06txdata\x02\0\x04\x04\0\x04send\x01\x05\x01\
j\x01\x02\x01\x01\x01@\0\0\x06\x04\0\x06keygen\x01\x07\x04\0\x09blockhash\x01\x07\
\x01@\x01\x04sizey\0\x03\x04\0\x04rent\x01\x08\x03\0*catscope:witbot/transaction\
processor@0.1.0\x05\0\x01B\x04\x04\0\x05error\x03\x01\x01h\0\x01@\x01\x04self\x01\
\0s\x04\0\x1d[method]error.to-debug-string\x01\x02\x03\0\x13wasi:io/error@0.2.6\x05\
\x01\x01B\x0a\x04\0\x08pollable\x03\x01\x01h\0\x01@\x01\x04self\x01\0\x7f\x04\0\x16\
[method]pollable.ready\x01\x02\x01@\x01\x04self\x01\x01\0\x04\0\x16[method]polla\
ble.block\x01\x03\x01p\x01\x01py\x01@\x01\x02in\x04\0\x05\x04\0\x04poll\x01\x06\x03\
\0\x12wasi:io/poll@0.2.6\x05\x02\x02\x03\0\x01\x05error\x02\x03\0\x02\x08pollabl\
e\x01B(\x02\x03\x02\x01\x03\x04\0\x05error\x03\0\0\x02\x03\x02\x01\x04\x04\0\x08\
pollable\x03\0\x02\x01i\x01\x01q\x02\x15last-operation-failed\x01\x04\0\x06close\
d\0\0\x04\0\x0cstream-error\x03\0\x05\x04\0\x0cinput-stream\x03\x01\x04\0\x0dout\
put-stream\x03\x01\x01h\x07\x01p}\x01j\x01\x0a\x01\x06\x01@\x02\x04self\x09\x03l\
enw\0\x0b\x04\0\x19[method]input-stream.read\x01\x0c\x04\0\"[method]input-stream\
.blocking-read\x01\x0c\x01j\x01w\x01\x06\x01@\x02\x04self\x09\x03lenw\0\x0d\x04\0\
\x19[method]input-stream.skip\x01\x0e\x04\0\"[method]input-stream.blocking-skip\x01\
\x0e\x01i\x03\x01@\x01\x04self\x09\0\x0f\x04\0\x1e[method]input-stream.subscribe\
\x01\x10\x01h\x08\x01@\x01\x04self\x11\0\x0d\x04\0![method]output-stream.check-w\
rite\x01\x12\x01j\0\x01\x06\x01@\x02\x04self\x11\x08contents\x0a\0\x13\x04\0\x1b\
[method]output-stream.write\x01\x14\x04\0.[method]output-stream.blocking-write-a\
nd-flush\x01\x14\x01@\x01\x04self\x11\0\x13\x04\0\x1b[method]output-stream.flush\
\x01\x15\x04\0$[method]output-stream.blocking-flush\x01\x15\x01@\x01\x04self\x11\
\0\x0f\x04\0\x1f[method]output-stream.subscribe\x01\x16\x01@\x02\x04self\x11\x03\
lenw\0\x13\x04\0\"[method]output-stream.write-zeroes\x01\x17\x04\05[method]outpu\
t-stream.blocking-write-zeroes-and-flush\x01\x17\x01@\x03\x04self\x11\x03src\x09\
\x03lenw\0\x0d\x04\0\x1c[method]output-stream.splice\x01\x18\x04\0%[method]outpu\
t-stream.blocking-splice\x01\x18\x03\0\x15wasi:io/streams@0.2.6\x05\x05\x02\x03\0\
\x03\x0cinput-stream\x02\x03\0\x03\x0doutput-stream\x01B\"\x02\x03\x02\x01\x06\x04\
\0\x0cinput-stream\x03\0\0\x02\x03\x02\x01\x07\x04\0\x0doutput-stream\x03\0\x02\x01\
m\x06\x07unknown\x0daccess-denied\x0dnot-supported\x10invalid-argument\x0dout-of\
-memory\x07timeout\x04\0\x0aerror-code\x03\0\x04\x04\0\x0csubscription\x03\x01\x01\
i\x06\x01@\0\0\x07\x04\0\x19[constructor]subscription\x01\x08\x01h\x06\x01e\x01y\
\x01@\x04\x04self\x09\x02idw\x06filtery\x05depthy\0\x0a\x04\0\x1e[method]subscri\
ption.subscribe\x01\x0b\x01@\x02\x04self\x09\x05subidy\x01\0\x04\0\x1b[method]su\
bscription.cancel\x01\x0c\x01p}\x01j\x01\x0d\x01\x05\x01e\x01\x0e\x01@\x01\x02id\
w\0\x0f\x04\0\x0daccount-by-id\x01\x10\x01@\x01\x06pubkey\x0d\0\x0f\x04\0\x11acc\
ount-by-pubkey\x01\x11\x01i\x03\x01o\x02\x07\x12\x01j\x01\x13\x01\x05\x01@\x03\x02\
idw\x06filtery\x05depthy\0\x14\x04\0\x09subscribe\x01\x15\x01@\x01\x06filterw\0\x14\
\x04\0\x14subscribebloomfilter\x01\x16\x01j\x01\x12\x01\x05\x01@\0\0\x17\x04\0\x04\
slot\x01\x18\x04\0\x09blockhash\x01\x18\x03\0\x1dcatscope:witbot/shooter@0.1.0\x05\
\x08\x01B\x0a\x01o\x02ss\x01p\0\x01@\0\0\x01\x04\0\x0fget-environment\x01\x02\x01\
ps\x01@\0\0\x03\x04\0\x0dget-arguments\x01\x04\x01ks\x01@\0\0\x05\x04\0\x0biniti\
al-cwd\x01\x06\x03\0\x1awasi:cli/environment@0.2.6\x05\x09\x01B\x03\x01j\0\0\x01\
@\x01\x06status\0\x01\0\x04\0\x04exit\x01\x01\x03\0\x13wasi:cli/exit@0.2.6\x05\x0a\
\x01B\x05\x02\x03\x02\x01\x06\x04\0\x0cinput-stream\x03\0\0\x01i\x01\x01@\0\0\x02\
\x04\0\x09get-stdin\x01\x03\x03\0\x14wasi:cli/stdin@0.2.6\x05\x0b\x01B\x05\x02\x03\
\x02\x01\x07\x04\0\x0doutput-stream\x03\0\0\x01i\x01\x01@\0\0\x02\x04\0\x0aget-s\
tdout\x01\x03\x03\0\x15wasi:cli/stdout@0.2.6\x05\x0c\x01B\x05\x02\x03\x02\x01\x07\
\x04\0\x0doutput-stream\x03\0\0\x01i\x01\x01@\0\0\x02\x04\0\x0aget-stderr\x01\x03\
\x03\0\x15wasi:cli/stderr@0.2.6\x05\x0d\x01B\x01\x04\0\x0eterminal-input\x03\x01\
\x03\0\x1dwasi:cli/terminal-input@0.2.6\x05\x0e\x01B\x01\x04\0\x0fterminal-outpu\
t\x03\x01\x03\0\x1ewasi:cli/terminal-output@0.2.6\x05\x0f\x02\x03\0\x0a\x0etermi\
nal-input\x01B\x06\x02\x03\x02\x01\x10\x04\0\x0eterminal-input\x03\0\0\x01i\x01\x01\
k\x02\x01@\0\0\x03\x04\0\x12get-terminal-stdin\x01\x04\x03\0\x1dwasi:cli/termina\
l-stdin@0.2.6\x05\x11\x02\x03\0\x0b\x0fterminal-output\x01B\x06\x02\x03\x02\x01\x12\
\x04\0\x0fterminal-output\x03\0\0\x01i\x01\x01k\x02\x01@\0\0\x03\x04\0\x13get-te\
rminal-stdout\x01\x04\x03\0\x1ewasi:cli/terminal-stdout@0.2.6\x05\x13\x01B\x06\x02\
\x03\x02\x01\x12\x04\0\x0fterminal-output\x03\0\0\x01i\x01\x01k\x02\x01@\0\0\x03\
\x04\0\x13get-terminal-stderr\x01\x04\x03\0\x1ewasi:cli/terminal-stderr@0.2.6\x05\
\x14\x01B\x0f\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\0\x01w\x04\0\x07instan\
t\x03\0\x02\x01w\x04\0\x08duration\x03\0\x04\x01@\0\0\x03\x04\0\x03now\x01\x06\x01\
@\0\0\x05\x04\0\x0aresolution\x01\x07\x01i\x01\x01@\x01\x04when\x03\0\x08\x04\0\x11\
subscribe-instant\x01\x09\x01@\x01\x04when\x05\0\x08\x04\0\x12subscribe-duration\
\x01\x0a\x03\0!wasi:clocks/monotonic-clock@0.2.6\x05\x15\x01B\x05\x01r\x02\x07se\
condsw\x0bnanosecondsy\x04\0\x08datetime\x03\0\0\x01@\0\0\x01\x04\0\x03now\x01\x02\
\x04\0\x0aresolution\x01\x02\x03\0\x1cwasi:clocks/wall-clock@0.2.6\x05\x16\x02\x03\
\0\x03\x05error\x02\x03\0\x10\x08datetime\x01Br\x02\x03\x02\x01\x06\x04\0\x0cinp\
ut-stream\x03\0\0\x02\x03\x02\x01\x07\x04\0\x0doutput-stream\x03\0\x02\x02\x03\x02\
\x01\x17\x04\0\x05error\x03\0\x04\x02\x03\x02\x01\x18\x04\0\x08datetime\x03\0\x06\
\x01w\x04\0\x08filesize\x03\0\x08\x01m\x08\x07unknown\x0cblock-device\x10charact\
er-device\x09directory\x04fifo\x0dsymbolic-link\x0cregular-file\x06socket\x04\0\x0f\
descriptor-type\x03\0\x0a\x01n\x06\x04read\x05write\x13file-integrity-sync\x13da\
ta-integrity-sync\x14requested-write-sync\x10mutate-directory\x04\0\x10descripto\
r-flags\x03\0\x0c\x01n\x01\x0esymlink-follow\x04\0\x0apath-flags\x03\0\x0e\x01n\x04\
\x06create\x09directory\x09exclusive\x08truncate\x04\0\x0aopen-flags\x03\0\x10\x01\
w\x04\0\x0alink-count\x03\0\x12\x01k\x07\x01r\x06\x04type\x0b\x0alink-count\x13\x04\
size\x09\x15data-access-timestamp\x14\x1bdata-modification-timestamp\x14\x17stat\
us-change-timestamp\x14\x04\0\x0fdescriptor-stat\x03\0\x15\x01q\x03\x09no-change\
\0\0\x03now\0\0\x09timestamp\x01\x07\0\x04\0\x0dnew-timestamp\x03\0\x17\x01r\x02\
\x04type\x0b\x04names\x04\0\x0fdirectory-entry\x03\0\x19\x01m%\x06access\x0bwoul\
d-block\x07already\x0ebad-descriptor\x04busy\x08deadlock\x05quota\x05exist\x0efi\
le-too-large\x15illegal-byte-sequence\x0bin-progress\x0binterrupted\x07invalid\x02\
io\x0cis-directory\x04loop\x0etoo-many-links\x0cmessage-size\x0dname-too-long\x09\
no-device\x08no-entry\x07no-lock\x13insufficient-memory\x12insufficient-space\x0d\
not-directory\x09not-empty\x0fnot-recoverable\x0bunsupported\x06no-tty\x0eno-suc\
h-device\x08overflow\x0dnot-permitted\x04pipe\x09read-only\x0cinvalid-seek\x0ete\
xt-file-busy\x0ccross-device\x04\0\x0aerror-code\x03\0\x1b\x01m\x06\x06normal\x0a\
sequential\x06random\x09will-need\x09dont-need\x08no-reuse\x04\0\x06advice\x03\0\
\x1d\x01r\x02\x05lowerw\x05upperw\x04\0\x13metadata-hash-value\x03\0\x1f\x04\0\x0a\
descriptor\x03\x01\x04\0\x16directory-entry-stream\x03\x01\x01h!\x01i\x01\x01j\x01\
$\x01\x1c\x01@\x02\x04self#\x06offset\x09\0%\x04\0\"[method]descriptor.read-via-\
stream\x01&\x01i\x03\x01j\x01'\x01\x1c\x01@\x02\x04self#\x06offset\x09\0(\x04\0#\
[method]descriptor.write-via-stream\x01)\x01@\x01\x04self#\0(\x04\0$[method]desc\
riptor.append-via-stream\x01*\x01j\0\x01\x1c\x01@\x04\x04self#\x06offset\x09\x06\
length\x09\x06advice\x1e\0+\x04\0\x19[method]descriptor.advise\x01,\x01@\x01\x04\
self#\0+\x04\0\x1c[method]descriptor.sync-data\x01-\x01j\x01\x0d\x01\x1c\x01@\x01\
\x04self#\0.\x04\0\x1c[method]descriptor.get-flags\x01/\x01j\x01\x0b\x01\x1c\x01\
@\x01\x04self#\00\x04\0\x1b[method]descriptor.get-type\x011\x01@\x02\x04self#\x04\
size\x09\0+\x04\0\x1b[method]descriptor.set-size\x012\x01@\x03\x04self#\x15data-\
access-timestamp\x18\x1bdata-modification-timestamp\x18\0+\x04\0\x1c[method]desc\
riptor.set-times\x013\x01p}\x01o\x024\x7f\x01j\x015\x01\x1c\x01@\x03\x04self#\x06\
length\x09\x06offset\x09\06\x04\0\x17[method]descriptor.read\x017\x01j\x01\x09\x01\
\x1c\x01@\x03\x04self#\x06buffer4\x06offset\x09\08\x04\0\x18[method]descriptor.w\
rite\x019\x01i\"\x01j\x01:\x01\x1c\x01@\x01\x04self#\0;\x04\0![method]descriptor\
.read-directory\x01<\x04\0\x17[method]descriptor.sync\x01-\x01@\x02\x04self#\x04\
paths\0+\x04\0&[method]descriptor.create-directory-at\x01=\x01j\x01\x16\x01\x1c\x01\
@\x01\x04self#\0>\x04\0\x17[method]descriptor.stat\x01?\x01@\x03\x04self#\x0apat\
h-flags\x0f\x04paths\0>\x04\0\x1a[method]descriptor.stat-at\x01@\x01@\x05\x04sel\
f#\x0apath-flags\x0f\x04paths\x15data-access-timestamp\x18\x1bdata-modification-\
timestamp\x18\0+\x04\0\x1f[method]descriptor.set-times-at\x01A\x01@\x05\x04self#\
\x0eold-path-flags\x0f\x08old-paths\x0enew-descriptor#\x08new-paths\0+\x04\0\x1a\
[method]descriptor.link-at\x01B\x01i!\x01j\x01\xc3\0\x01\x1c\x01@\x05\x04self#\x0a\
path-flags\x0f\x04paths\x0aopen-flags\x11\x05flags\x0d\0\xc4\0\x04\0\x1a[method]\
descriptor.open-at\x01E\x01j\x01s\x01\x1c\x01@\x02\x04self#\x04paths\0\xc6\0\x04\
\0\x1e[method]descriptor.readlink-at\x01G\x04\0&[method]descriptor.remove-direct\
ory-at\x01=\x01@\x04\x04self#\x08old-paths\x0enew-descriptor#\x08new-paths\0+\x04\
\0\x1c[method]descriptor.rename-at\x01H\x01@\x03\x04self#\x08old-paths\x08new-pa\
ths\0+\x04\0\x1d[method]descriptor.symlink-at\x01I\x04\0![method]descriptor.unli\
nk-file-at\x01=\x01@\x02\x04self#\x05other#\0\x7f\x04\0![method]descriptor.is-sa\
me-object\x01J\x01j\x01\x20\x01\x1c\x01@\x01\x04self#\0\xcb\0\x04\0\x20[method]d\
escriptor.metadata-hash\x01L\x01@\x03\x04self#\x0apath-flags\x0f\x04paths\0\xcb\0\
\x04\0#[method]descriptor.metadata-hash-at\x01M\x01h\"\x01k\x1a\x01j\x01\xcf\0\x01\
\x1c\x01@\x01\x04self\xce\0\0\xd0\0\x04\03[method]directory-entry-stream.read-di\
rectory-entry\x01Q\x01h\x05\x01k\x1c\x01@\x01\x03err\xd2\0\0\xd3\0\x04\0\x15file\
system-error-code\x01T\x03\0\x1bwasi:filesystem/types@0.2.6\x05\x19\x02\x03\0\x11\
\x0adescriptor\x01B\x07\x02\x03\x02\x01\x1a\x04\0\x0adescriptor\x03\0\0\x01i\x01\
\x01o\x02\x02s\x01p\x03\x01@\0\0\x04\x04\0\x0fget-directories\x01\x05\x03\0\x1ew\
asi:filesystem/preopens@0.2.6\x05\x1b\x01B\x17\x02\x03\x02\x01\x03\x04\0\x05erro\
r\x03\0\0\x04\0\x07network\x03\x01\x01m\x15\x07unknown\x0daccess-denied\x0dnot-s\
upported\x10invalid-argument\x0dout-of-memory\x07timeout\x14concurrency-conflict\
\x0fnot-in-progress\x0bwould-block\x0dinvalid-state\x10new-socket-limit\x14addre\
ss-not-bindable\x0eaddress-in-use\x12remote-unreachable\x12connection-refused\x10\
connection-reset\x12connection-aborted\x12datagram-too-large\x11name-unresolvabl\
e\x1atemporary-resolver-failure\x1apermanent-resolver-failure\x04\0\x0aerror-cod\
e\x03\0\x03\x01m\x02\x04ipv4\x04ipv6\x04\0\x11ip-address-family\x03\0\x05\x01o\x04\
}}}}\x04\0\x0cipv4-address\x03\0\x07\x01o\x08{{{{{{{{\x04\0\x0cipv6-address\x03\0\
\x09\x01q\x02\x04ipv4\x01\x08\0\x04ipv6\x01\x0a\0\x04\0\x0aip-address\x03\0\x0b\x01\
r\x02\x04port{\x07address\x08\x04\0\x13ipv4-socket-address\x03\0\x0d\x01r\x04\x04\
port{\x09flow-infoy\x07address\x0a\x08scope-idy\x04\0\x13ipv6-socket-address\x03\
\0\x0f\x01q\x02\x04ipv4\x01\x0e\0\x04ipv6\x01\x10\0\x04\0\x11ip-socket-address\x03\
\0\x11\x01h\x01\x01k\x04\x01@\x01\x03err\x13\0\x14\x04\0\x12network-error-code\x01\
\x15\x03\0\x1awasi:sockets/network@0.2.6\x05\x1c\x02\x03\0\x13\x07network\x01B\x05\
\x02\x03\x02\x01\x1d\x04\0\x07network\x03\0\0\x01i\x01\x01@\0\0\x02\x04\0\x10ins\
tance-network\x01\x03\x03\0#wasi:sockets/instance-network@0.2.6\x05\x1e\x02\x03\0\
\x13\x0aerror-code\x02\x03\0\x13\x11ip-socket-address\x02\x03\0\x13\x11ip-addres\
s-family\x01BD\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\0\x02\x03\x02\x01\x1d\
\x04\0\x07network\x03\0\x02\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x04\x02\
\x03\x02\x01\x20\x04\0\x11ip-socket-address\x03\0\x06\x02\x03\x02\x01!\x04\0\x11\
ip-address-family\x03\0\x08\x01p}\x01r\x02\x04data\x0a\x0eremote-address\x07\x04\
\0\x11incoming-datagram\x03\0\x0b\x01k\x07\x01r\x02\x04data\x0a\x0eremote-addres\
s\x0d\x04\0\x11outgoing-datagram\x03\0\x0e\x04\0\x0audp-socket\x03\x01\x04\0\x18\
incoming-datagram-stream\x03\x01\x04\0\x18outgoing-datagram-stream\x03\x01\x01h\x10\
\x01h\x03\x01j\0\x01\x05\x01@\x03\x04self\x13\x07network\x14\x0dlocal-address\x07\
\0\x15\x04\0\x1d[method]udp-socket.start-bind\x01\x16\x01@\x01\x04self\x13\0\x15\
\x04\0\x1e[method]udp-socket.finish-bind\x01\x17\x01i\x11\x01i\x12\x01o\x02\x18\x19\
\x01j\x01\x1a\x01\x05\x01@\x02\x04self\x13\x0eremote-address\x0d\0\x1b\x04\0\x19\
[method]udp-socket.stream\x01\x1c\x01j\x01\x07\x01\x05\x01@\x01\x04self\x13\0\x1d\
\x04\0\x20[method]udp-socket.local-address\x01\x1e\x04\0![method]udp-socket.remo\
te-address\x01\x1e\x01@\x01\x04self\x13\0\x09\x04\0![method]udp-socket.address-f\
amily\x01\x1f\x01j\x01}\x01\x05\x01@\x01\x04self\x13\0\x20\x04\0$[method]udp-soc\
ket.unicast-hop-limit\x01!\x01@\x02\x04self\x13\x05value}\0\x15\x04\0([method]ud\
p-socket.set-unicast-hop-limit\x01\"\x01j\x01w\x01\x05\x01@\x01\x04self\x13\0#\x04\
\0&[method]udp-socket.receive-buffer-size\x01$\x01@\x02\x04self\x13\x05valuew\0\x15\
\x04\0*[method]udp-socket.set-receive-buffer-size\x01%\x04\0#[method]udp-socket.\
send-buffer-size\x01$\x04\0'[method]udp-socket.set-send-buffer-size\x01%\x01i\x01\
\x01@\x01\x04self\x13\0&\x04\0\x1c[method]udp-socket.subscribe\x01'\x01h\x11\x01\
p\x0c\x01j\x01)\x01\x05\x01@\x02\x04self(\x0bmax-resultsw\0*\x04\0([method]incom\
ing-datagram-stream.receive\x01+\x01@\x01\x04self(\0&\x04\0*[method]incoming-dat\
agram-stream.subscribe\x01,\x01h\x12\x01@\x01\x04self-\0#\x04\0+[method]outgoing\
-datagram-stream.check-send\x01.\x01p\x0f\x01@\x02\x04self-\x09datagrams/\0#\x04\
\0%[method]outgoing-datagram-stream.send\x010\x01@\x01\x04self-\0&\x04\0*[method\
]outgoing-datagram-stream.subscribe\x011\x03\0\x16wasi:sockets/udp@0.2.6\x05\"\x02\
\x03\0\x15\x0audp-socket\x01B\x0c\x02\x03\x02\x01\x1d\x04\0\x07network\x03\0\0\x02\
\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x02\x02\x03\x02\x01!\x04\0\x11ip-addr\
ess-family\x03\0\x04\x02\x03\x02\x01#\x04\0\x0audp-socket\x03\0\x06\x01i\x07\x01\
j\x01\x08\x01\x03\x01@\x01\x0eaddress-family\x05\0\x09\x04\0\x11create-udp-socke\
t\x01\x0a\x03\0$wasi:sockets/udp-create-socket@0.2.6\x05$\x02\x03\0\x0f\x08durat\
ion\x01BT\x02\x03\x02\x01\x06\x04\0\x0cinput-stream\x03\0\0\x02\x03\x02\x01\x07\x04\
\0\x0doutput-stream\x03\0\x02\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\x04\x02\
\x03\x02\x01%\x04\0\x08duration\x03\0\x06\x02\x03\x02\x01\x1d\x04\0\x07network\x03\
\0\x08\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x0a\x02\x03\x02\x01\x20\x04\
\0\x11ip-socket-address\x03\0\x0c\x02\x03\x02\x01!\x04\0\x11ip-address-family\x03\
\0\x0e\x01m\x03\x07receive\x04send\x04both\x04\0\x0dshutdown-type\x03\0\x10\x04\0\
\x0atcp-socket\x03\x01\x01h\x12\x01h\x09\x01j\0\x01\x0b\x01@\x03\x04self\x13\x07\
network\x14\x0dlocal-address\x0d\0\x15\x04\0\x1d[method]tcp-socket.start-bind\x01\
\x16\x01@\x01\x04self\x13\0\x15\x04\0\x1e[method]tcp-socket.finish-bind\x01\x17\x01\
@\x03\x04self\x13\x07network\x14\x0eremote-address\x0d\0\x15\x04\0\x20[method]tc\
p-socket.start-connect\x01\x18\x01i\x01\x01i\x03\x01o\x02\x19\x1a\x01j\x01\x1b\x01\
\x0b\x01@\x01\x04self\x13\0\x1c\x04\0![method]tcp-socket.finish-connect\x01\x1d\x04\
\0\x1f[method]tcp-socket.start-listen\x01\x17\x04\0\x20[method]tcp-socket.finish\
-listen\x01\x17\x01i\x12\x01o\x03\x1e\x19\x1a\x01j\x01\x1f\x01\x0b\x01@\x01\x04s\
elf\x13\0\x20\x04\0\x19[method]tcp-socket.accept\x01!\x01j\x01\x0d\x01\x0b\x01@\x01\
\x04self\x13\0\"\x04\0\x20[method]tcp-socket.local-address\x01#\x04\0![method]tc\
p-socket.remote-address\x01#\x01@\x01\x04self\x13\0\x7f\x04\0\x1f[method]tcp-soc\
ket.is-listening\x01$\x01@\x01\x04self\x13\0\x0f\x04\0![method]tcp-socket.addres\
s-family\x01%\x01@\x02\x04self\x13\x05valuew\0\x15\x04\0*[method]tcp-socket.set-\
listen-backlog-size\x01&\x01j\x01\x7f\x01\x0b\x01@\x01\x04self\x13\0'\x04\0%[met\
hod]tcp-socket.keep-alive-enabled\x01(\x01@\x02\x04self\x13\x05value\x7f\0\x15\x04\
\0)[method]tcp-socket.set-keep-alive-enabled\x01)\x01j\x01\x07\x01\x0b\x01@\x01\x04\
self\x13\0*\x04\0'[method]tcp-socket.keep-alive-idle-time\x01+\x01@\x02\x04self\x13\
\x05value\x07\0\x15\x04\0+[method]tcp-socket.set-keep-alive-idle-time\x01,\x04\0\
&[method]tcp-socket.keep-alive-interval\x01+\x04\0*[method]tcp-socket.set-keep-a\
live-interval\x01,\x01j\x01y\x01\x0b\x01@\x01\x04self\x13\0-\x04\0#[method]tcp-s\
ocket.keep-alive-count\x01.\x01@\x02\x04self\x13\x05valuey\0\x15\x04\0'[method]t\
cp-socket.set-keep-alive-count\x01/\x01j\x01}\x01\x0b\x01@\x01\x04self\x13\00\x04\
\0\x1c[method]tcp-socket.hop-limit\x011\x01@\x02\x04self\x13\x05value}\0\x15\x04\
\0\x20[method]tcp-socket.set-hop-limit\x012\x01j\x01w\x01\x0b\x01@\x01\x04self\x13\
\03\x04\0&[method]tcp-socket.receive-buffer-size\x014\x04\0*[method]tcp-socket.s\
et-receive-buffer-size\x01&\x04\0#[method]tcp-socket.send-buffer-size\x014\x04\0\
'[method]tcp-socket.set-send-buffer-size\x01&\x01i\x05\x01@\x01\x04self\x13\05\x04\
\0\x1c[method]tcp-socket.subscribe\x016\x01@\x02\x04self\x13\x0dshutdown-type\x11\
\0\x15\x04\0\x1b[method]tcp-socket.shutdown\x017\x03\0\x16wasi:sockets/tcp@0.2.6\
\x05&\x02\x03\0\x17\x0atcp-socket\x01B\x0c\x02\x03\x02\x01\x1d\x04\0\x07network\x03\
\0\0\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x02\x02\x03\x02\x01!\x04\0\x11\
ip-address-family\x03\0\x04\x02\x03\x02\x01'\x04\0\x0atcp-socket\x03\0\x06\x01i\x07\
\x01j\x01\x08\x01\x03\x01@\x01\x0eaddress-family\x05\0\x09\x04\0\x11create-tcp-s\
ocket\x01\x0a\x03\0$wasi:sockets/tcp-create-socket@0.2.6\x05(\x02\x03\0\x13\x0ai\
p-address\x01B\x16\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\0\x02\x03\x02\x01\
\x1d\x04\0\x07network\x03\0\x02\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x04\
\x02\x03\x02\x01)\x04\0\x0aip-address\x03\0\x06\x04\0\x16resolve-address-stream\x03\
\x01\x01h\x08\x01k\x07\x01j\x01\x0a\x01\x05\x01@\x01\x04self\x09\0\x0b\x04\03[me\
thod]resolve-address-stream.resolve-next-address\x01\x0c\x01i\x01\x01@\x01\x04se\
lf\x09\0\x0d\x04\0([method]resolve-address-stream.subscribe\x01\x0e\x01h\x03\x01\
i\x08\x01j\x01\x10\x01\x05\x01@\x02\x07network\x0f\x04names\0\x11\x04\0\x11resol\
ve-addresses\x01\x12\x03\0!wasi:sockets/ip-name-lookup@0.2.6\x05*\x01B\x05\x01p}\
\x01@\x01\x03lenw\0\0\x04\0\x10get-random-bytes\x01\x01\x01@\0\0w\x04\0\x0eget-r\
andom-u64\x01\x02\x03\0\x18wasi:random/random@0.2.6\x05+\x01B\x05\x01p}\x01@\x01\
\x03lenw\0\0\x04\0\x19get-insecure-random-bytes\x01\x01\x01@\0\0w\x04\0\x17get-i\
nsecure-random-u64\x01\x02\x03\0\x1awasi:random/insecure@0.2.6\x05,\x01B\x03\x01\
o\x02ww\x01@\0\0\0\x04\0\x0dinsecure-seed\x01\x01\x03\0\x1fwasi:random/insecure-\
seed@0.2.6\x05-\x01B\x09\x01p}\x01@\x04\x09signature\0\x04slotw\x06status}\x08tx\
resulty\x01\0\x04\0\x0dtransactionv1\x01\x01\x01@\x06\x02idw\x04slotw\x08lamport\
sw\x04rentw\x05ownerw\x08datasizey\x01\0\x04\0\x09accountv1\x01\x02\x01@\x06\x02\
idw\x04slotw\x08lamportsw\x04mintw\x05ownerw\x07balancew\x01\0\x04\0\x07tokenv1\x01\
\x03\x01@\x02\x04slotw\x06status}\x01\0\x04\0\x04slot\x01\x04\x04\0\x1dcatscope:\
witbot/updater@0.1.0\x05.\x01B\x03\x01j\0\0\x01@\0\0\0\x04\0\x03run\x01\x01\x04\0\
\x12wasi:cli/run@0.2.6\x05/\x04\0'catscope:witbot/catscopevalidator@0.1.0\x04\0\x0b\
\x17\x01\0\x11catscopevalidator\x03\0\0\0G\x09producers\x01\x0cprocessed-by\x02\x0d\
wit-component\x070.236.1\x10wit-bindgen-rust\x060.44.0";
        };
    };
}
#[doc(inline)]
pub use __export_catscopevalidator_impl as export;
export!(Stub);
#[rustfmt::skip]
#[cfg(target_arch = "wasm32")]
#[unsafe(
    link_section = "component-type:wit-bindgen:0.44.0:catscope:witbot@0.1.0:catscopevalidator-with-all-of-its-exports-removed:encoded world"
)]
#[doc(hidden)]
#[allow(clippy::octal_escapes)]
pub static __WIT_BINDGEN_COMPONENT_TYPE: [u8; 11676] = *b"\
\0asm\x0d\0\x01\0\0\x19\x16wit-component-encoding\x04\0\x07\xf4Y\x01A\x02\x01AK\x01\
B\x0d\x01m\x06\x07unknown\x0daccess-denied\x0dnot-supported\x10invalid-argument\x0d\
out-of-memory\x07timeout\x04\0\x0aerror-code\x03\0\0\x01p}\x01j\x01w\x01\x01\x01\
e\x01\x03\x01@\x02\x09signature\x02\x06txdata\x02\0\x04\x04\0\x04send\x01\x05\x01\
j\x01\x02\x01\x01\x01@\0\0\x06\x04\0\x06keygen\x01\x07\x04\0\x09blockhash\x01\x07\
\x01@\x01\x04sizey\0\x03\x04\0\x04rent\x01\x08\x03\0*catscope:witbot/transaction\
processor@0.1.0\x05\0\x01B\x04\x04\0\x05error\x03\x01\x01h\0\x01@\x01\x04self\x01\
\0s\x04\0\x1d[method]error.to-debug-string\x01\x02\x03\0\x13wasi:io/error@0.2.6\x05\
\x01\x01B\x0a\x04\0\x08pollable\x03\x01\x01h\0\x01@\x01\x04self\x01\0\x7f\x04\0\x16\
[method]pollable.ready\x01\x02\x01@\x01\x04self\x01\x01\0\x04\0\x16[method]polla\
ble.block\x01\x03\x01p\x01\x01py\x01@\x01\x02in\x04\0\x05\x04\0\x04poll\x01\x06\x03\
\0\x12wasi:io/poll@0.2.6\x05\x02\x02\x03\0\x01\x05error\x02\x03\0\x02\x08pollabl\
e\x01B(\x02\x03\x02\x01\x03\x04\0\x05error\x03\0\0\x02\x03\x02\x01\x04\x04\0\x08\
pollable\x03\0\x02\x01i\x01\x01q\x02\x15last-operation-failed\x01\x04\0\x06close\
d\0\0\x04\0\x0cstream-error\x03\0\x05\x04\0\x0cinput-stream\x03\x01\x04\0\x0dout\
put-stream\x03\x01\x01h\x07\x01p}\x01j\x01\x0a\x01\x06\x01@\x02\x04self\x09\x03l\
enw\0\x0b\x04\0\x19[method]input-stream.read\x01\x0c\x04\0\"[method]input-stream\
.blocking-read\x01\x0c\x01j\x01w\x01\x06\x01@\x02\x04self\x09\x03lenw\0\x0d\x04\0\
\x19[method]input-stream.skip\x01\x0e\x04\0\"[method]input-stream.blocking-skip\x01\
\x0e\x01i\x03\x01@\x01\x04self\x09\0\x0f\x04\0\x1e[method]input-stream.subscribe\
\x01\x10\x01h\x08\x01@\x01\x04self\x11\0\x0d\x04\0![method]output-stream.check-w\
rite\x01\x12\x01j\0\x01\x06\x01@\x02\x04self\x11\x08contents\x0a\0\x13\x04\0\x1b\
[method]output-stream.write\x01\x14\x04\0.[method]output-stream.blocking-write-a\
nd-flush\x01\x14\x01@\x01\x04self\x11\0\x13\x04\0\x1b[method]output-stream.flush\
\x01\x15\x04\0$[method]output-stream.blocking-flush\x01\x15\x01@\x01\x04self\x11\
\0\x0f\x04\0\x1f[method]output-stream.subscribe\x01\x16\x01@\x02\x04self\x11\x03\
lenw\0\x13\x04\0\"[method]output-stream.write-zeroes\x01\x17\x04\05[method]outpu\
t-stream.blocking-write-zeroes-and-flush\x01\x17\x01@\x03\x04self\x11\x03src\x09\
\x03lenw\0\x0d\x04\0\x1c[method]output-stream.splice\x01\x18\x04\0%[method]outpu\
t-stream.blocking-splice\x01\x18\x03\0\x15wasi:io/streams@0.2.6\x05\x05\x02\x03\0\
\x03\x0cinput-stream\x02\x03\0\x03\x0doutput-stream\x01B\"\x02\x03\x02\x01\x06\x04\
\0\x0cinput-stream\x03\0\0\x02\x03\x02\x01\x07\x04\0\x0doutput-stream\x03\0\x02\x01\
m\x06\x07unknown\x0daccess-denied\x0dnot-supported\x10invalid-argument\x0dout-of\
-memory\x07timeout\x04\0\x0aerror-code\x03\0\x04\x04\0\x0csubscription\x03\x01\x01\
i\x06\x01@\0\0\x07\x04\0\x19[constructor]subscription\x01\x08\x01h\x06\x01e\x01y\
\x01@\x04\x04self\x09\x02idw\x06filtery\x05depthy\0\x0a\x04\0\x1e[method]subscri\
ption.subscribe\x01\x0b\x01@\x02\x04self\x09\x05subidy\x01\0\x04\0\x1b[method]su\
bscription.cancel\x01\x0c\x01p}\x01j\x01\x0d\x01\x05\x01e\x01\x0e\x01@\x01\x02id\
w\0\x0f\x04\0\x0daccount-by-id\x01\x10\x01@\x01\x06pubkey\x0d\0\x0f\x04\0\x11acc\
ount-by-pubkey\x01\x11\x01i\x03\x01o\x02\x07\x12\x01j\x01\x13\x01\x05\x01@\x03\x02\
idw\x06filtery\x05depthy\0\x14\x04\0\x09subscribe\x01\x15\x01@\x01\x06filterw\0\x14\
\x04\0\x14subscribebloomfilter\x01\x16\x01j\x01\x12\x01\x05\x01@\0\0\x17\x04\0\x04\
slot\x01\x18\x04\0\x09blockhash\x01\x18\x03\0\x1dcatscope:witbot/shooter@0.1.0\x05\
\x08\x01B\x0a\x01o\x02ss\x01p\0\x01@\0\0\x01\x04\0\x0fget-environment\x01\x02\x01\
ps\x01@\0\0\x03\x04\0\x0dget-arguments\x01\x04\x01ks\x01@\0\0\x05\x04\0\x0biniti\
al-cwd\x01\x06\x03\0\x1awasi:cli/environment@0.2.6\x05\x09\x01B\x03\x01j\0\0\x01\
@\x01\x06status\0\x01\0\x04\0\x04exit\x01\x01\x03\0\x13wasi:cli/exit@0.2.6\x05\x0a\
\x01B\x05\x02\x03\x02\x01\x06\x04\0\x0cinput-stream\x03\0\0\x01i\x01\x01@\0\0\x02\
\x04\0\x09get-stdin\x01\x03\x03\0\x14wasi:cli/stdin@0.2.6\x05\x0b\x01B\x05\x02\x03\
\x02\x01\x07\x04\0\x0doutput-stream\x03\0\0\x01i\x01\x01@\0\0\x02\x04\0\x0aget-s\
tdout\x01\x03\x03\0\x15wasi:cli/stdout@0.2.6\x05\x0c\x01B\x05\x02\x03\x02\x01\x07\
\x04\0\x0doutput-stream\x03\0\0\x01i\x01\x01@\0\0\x02\x04\0\x0aget-stderr\x01\x03\
\x03\0\x15wasi:cli/stderr@0.2.6\x05\x0d\x01B\x01\x04\0\x0eterminal-input\x03\x01\
\x03\0\x1dwasi:cli/terminal-input@0.2.6\x05\x0e\x01B\x01\x04\0\x0fterminal-outpu\
t\x03\x01\x03\0\x1ewasi:cli/terminal-output@0.2.6\x05\x0f\x02\x03\0\x0a\x0etermi\
nal-input\x01B\x06\x02\x03\x02\x01\x10\x04\0\x0eterminal-input\x03\0\0\x01i\x01\x01\
k\x02\x01@\0\0\x03\x04\0\x12get-terminal-stdin\x01\x04\x03\0\x1dwasi:cli/termina\
l-stdin@0.2.6\x05\x11\x02\x03\0\x0b\x0fterminal-output\x01B\x06\x02\x03\x02\x01\x12\
\x04\0\x0fterminal-output\x03\0\0\x01i\x01\x01k\x02\x01@\0\0\x03\x04\0\x13get-te\
rminal-stdout\x01\x04\x03\0\x1ewasi:cli/terminal-stdout@0.2.6\x05\x13\x01B\x06\x02\
\x03\x02\x01\x12\x04\0\x0fterminal-output\x03\0\0\x01i\x01\x01k\x02\x01@\0\0\x03\
\x04\0\x13get-terminal-stderr\x01\x04\x03\0\x1ewasi:cli/terminal-stderr@0.2.6\x05\
\x14\x01B\x0f\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\0\x01w\x04\0\x07instan\
t\x03\0\x02\x01w\x04\0\x08duration\x03\0\x04\x01@\0\0\x03\x04\0\x03now\x01\x06\x01\
@\0\0\x05\x04\0\x0aresolution\x01\x07\x01i\x01\x01@\x01\x04when\x03\0\x08\x04\0\x11\
subscribe-instant\x01\x09\x01@\x01\x04when\x05\0\x08\x04\0\x12subscribe-duration\
\x01\x0a\x03\0!wasi:clocks/monotonic-clock@0.2.6\x05\x15\x01B\x05\x01r\x02\x07se\
condsw\x0bnanosecondsy\x04\0\x08datetime\x03\0\0\x01@\0\0\x01\x04\0\x03now\x01\x02\
\x04\0\x0aresolution\x01\x02\x03\0\x1cwasi:clocks/wall-clock@0.2.6\x05\x16\x02\x03\
\0\x03\x05error\x02\x03\0\x10\x08datetime\x01Br\x02\x03\x02\x01\x06\x04\0\x0cinp\
ut-stream\x03\0\0\x02\x03\x02\x01\x07\x04\0\x0doutput-stream\x03\0\x02\x02\x03\x02\
\x01\x17\x04\0\x05error\x03\0\x04\x02\x03\x02\x01\x18\x04\0\x08datetime\x03\0\x06\
\x01w\x04\0\x08filesize\x03\0\x08\x01m\x08\x07unknown\x0cblock-device\x10charact\
er-device\x09directory\x04fifo\x0dsymbolic-link\x0cregular-file\x06socket\x04\0\x0f\
descriptor-type\x03\0\x0a\x01n\x06\x04read\x05write\x13file-integrity-sync\x13da\
ta-integrity-sync\x14requested-write-sync\x10mutate-directory\x04\0\x10descripto\
r-flags\x03\0\x0c\x01n\x01\x0esymlink-follow\x04\0\x0apath-flags\x03\0\x0e\x01n\x04\
\x06create\x09directory\x09exclusive\x08truncate\x04\0\x0aopen-flags\x03\0\x10\x01\
w\x04\0\x0alink-count\x03\0\x12\x01k\x07\x01r\x06\x04type\x0b\x0alink-count\x13\x04\
size\x09\x15data-access-timestamp\x14\x1bdata-modification-timestamp\x14\x17stat\
us-change-timestamp\x14\x04\0\x0fdescriptor-stat\x03\0\x15\x01q\x03\x09no-change\
\0\0\x03now\0\0\x09timestamp\x01\x07\0\x04\0\x0dnew-timestamp\x03\0\x17\x01r\x02\
\x04type\x0b\x04names\x04\0\x0fdirectory-entry\x03\0\x19\x01m%\x06access\x0bwoul\
d-block\x07already\x0ebad-descriptor\x04busy\x08deadlock\x05quota\x05exist\x0efi\
le-too-large\x15illegal-byte-sequence\x0bin-progress\x0binterrupted\x07invalid\x02\
io\x0cis-directory\x04loop\x0etoo-many-links\x0cmessage-size\x0dname-too-long\x09\
no-device\x08no-entry\x07no-lock\x13insufficient-memory\x12insufficient-space\x0d\
not-directory\x09not-empty\x0fnot-recoverable\x0bunsupported\x06no-tty\x0eno-suc\
h-device\x08overflow\x0dnot-permitted\x04pipe\x09read-only\x0cinvalid-seek\x0ete\
xt-file-busy\x0ccross-device\x04\0\x0aerror-code\x03\0\x1b\x01m\x06\x06normal\x0a\
sequential\x06random\x09will-need\x09dont-need\x08no-reuse\x04\0\x06advice\x03\0\
\x1d\x01r\x02\x05lowerw\x05upperw\x04\0\x13metadata-hash-value\x03\0\x1f\x04\0\x0a\
descriptor\x03\x01\x04\0\x16directory-entry-stream\x03\x01\x01h!\x01i\x01\x01j\x01\
$\x01\x1c\x01@\x02\x04self#\x06offset\x09\0%\x04\0\"[method]descriptor.read-via-\
stream\x01&\x01i\x03\x01j\x01'\x01\x1c\x01@\x02\x04self#\x06offset\x09\0(\x04\0#\
[method]descriptor.write-via-stream\x01)\x01@\x01\x04self#\0(\x04\0$[method]desc\
riptor.append-via-stream\x01*\x01j\0\x01\x1c\x01@\x04\x04self#\x06offset\x09\x06\
length\x09\x06advice\x1e\0+\x04\0\x19[method]descriptor.advise\x01,\x01@\x01\x04\
self#\0+\x04\0\x1c[method]descriptor.sync-data\x01-\x01j\x01\x0d\x01\x1c\x01@\x01\
\x04self#\0.\x04\0\x1c[method]descriptor.get-flags\x01/\x01j\x01\x0b\x01\x1c\x01\
@\x01\x04self#\00\x04\0\x1b[method]descriptor.get-type\x011\x01@\x02\x04self#\x04\
size\x09\0+\x04\0\x1b[method]descriptor.set-size\x012\x01@\x03\x04self#\x15data-\
access-timestamp\x18\x1bdata-modification-timestamp\x18\0+\x04\0\x1c[method]desc\
riptor.set-times\x013\x01p}\x01o\x024\x7f\x01j\x015\x01\x1c\x01@\x03\x04self#\x06\
length\x09\x06offset\x09\06\x04\0\x17[method]descriptor.read\x017\x01j\x01\x09\x01\
\x1c\x01@\x03\x04self#\x06buffer4\x06offset\x09\08\x04\0\x18[method]descriptor.w\
rite\x019\x01i\"\x01j\x01:\x01\x1c\x01@\x01\x04self#\0;\x04\0![method]descriptor\
.read-directory\x01<\x04\0\x17[method]descriptor.sync\x01-\x01@\x02\x04self#\x04\
paths\0+\x04\0&[method]descriptor.create-directory-at\x01=\x01j\x01\x16\x01\x1c\x01\
@\x01\x04self#\0>\x04\0\x17[method]descriptor.stat\x01?\x01@\x03\x04self#\x0apat\
h-flags\x0f\x04paths\0>\x04\0\x1a[method]descriptor.stat-at\x01@\x01@\x05\x04sel\
f#\x0apath-flags\x0f\x04paths\x15data-access-timestamp\x18\x1bdata-modification-\
timestamp\x18\0+\x04\0\x1f[method]descriptor.set-times-at\x01A\x01@\x05\x04self#\
\x0eold-path-flags\x0f\x08old-paths\x0enew-descriptor#\x08new-paths\0+\x04\0\x1a\
[method]descriptor.link-at\x01B\x01i!\x01j\x01\xc3\0\x01\x1c\x01@\x05\x04self#\x0a\
path-flags\x0f\x04paths\x0aopen-flags\x11\x05flags\x0d\0\xc4\0\x04\0\x1a[method]\
descriptor.open-at\x01E\x01j\x01s\x01\x1c\x01@\x02\x04self#\x04paths\0\xc6\0\x04\
\0\x1e[method]descriptor.readlink-at\x01G\x04\0&[method]descriptor.remove-direct\
ory-at\x01=\x01@\x04\x04self#\x08old-paths\x0enew-descriptor#\x08new-paths\0+\x04\
\0\x1c[method]descriptor.rename-at\x01H\x01@\x03\x04self#\x08old-paths\x08new-pa\
ths\0+\x04\0\x1d[method]descriptor.symlink-at\x01I\x04\0![method]descriptor.unli\
nk-file-at\x01=\x01@\x02\x04self#\x05other#\0\x7f\x04\0![method]descriptor.is-sa\
me-object\x01J\x01j\x01\x20\x01\x1c\x01@\x01\x04self#\0\xcb\0\x04\0\x20[method]d\
escriptor.metadata-hash\x01L\x01@\x03\x04self#\x0apath-flags\x0f\x04paths\0\xcb\0\
\x04\0#[method]descriptor.metadata-hash-at\x01M\x01h\"\x01k\x1a\x01j\x01\xcf\0\x01\
\x1c\x01@\x01\x04self\xce\0\0\xd0\0\x04\03[method]directory-entry-stream.read-di\
rectory-entry\x01Q\x01h\x05\x01k\x1c\x01@\x01\x03err\xd2\0\0\xd3\0\x04\0\x15file\
system-error-code\x01T\x03\0\x1bwasi:filesystem/types@0.2.6\x05\x19\x02\x03\0\x11\
\x0adescriptor\x01B\x07\x02\x03\x02\x01\x1a\x04\0\x0adescriptor\x03\0\0\x01i\x01\
\x01o\x02\x02s\x01p\x03\x01@\0\0\x04\x04\0\x0fget-directories\x01\x05\x03\0\x1ew\
asi:filesystem/preopens@0.2.6\x05\x1b\x01B\x17\x02\x03\x02\x01\x03\x04\0\x05erro\
r\x03\0\0\x04\0\x07network\x03\x01\x01m\x15\x07unknown\x0daccess-denied\x0dnot-s\
upported\x10invalid-argument\x0dout-of-memory\x07timeout\x14concurrency-conflict\
\x0fnot-in-progress\x0bwould-block\x0dinvalid-state\x10new-socket-limit\x14addre\
ss-not-bindable\x0eaddress-in-use\x12remote-unreachable\x12connection-refused\x10\
connection-reset\x12connection-aborted\x12datagram-too-large\x11name-unresolvabl\
e\x1atemporary-resolver-failure\x1apermanent-resolver-failure\x04\0\x0aerror-cod\
e\x03\0\x03\x01m\x02\x04ipv4\x04ipv6\x04\0\x11ip-address-family\x03\0\x05\x01o\x04\
}}}}\x04\0\x0cipv4-address\x03\0\x07\x01o\x08{{{{{{{{\x04\0\x0cipv6-address\x03\0\
\x09\x01q\x02\x04ipv4\x01\x08\0\x04ipv6\x01\x0a\0\x04\0\x0aip-address\x03\0\x0b\x01\
r\x02\x04port{\x07address\x08\x04\0\x13ipv4-socket-address\x03\0\x0d\x01r\x04\x04\
port{\x09flow-infoy\x07address\x0a\x08scope-idy\x04\0\x13ipv6-socket-address\x03\
\0\x0f\x01q\x02\x04ipv4\x01\x0e\0\x04ipv6\x01\x10\0\x04\0\x11ip-socket-address\x03\
\0\x11\x01h\x01\x01k\x04\x01@\x01\x03err\x13\0\x14\x04\0\x12network-error-code\x01\
\x15\x03\0\x1awasi:sockets/network@0.2.6\x05\x1c\x02\x03\0\x13\x07network\x01B\x05\
\x02\x03\x02\x01\x1d\x04\0\x07network\x03\0\0\x01i\x01\x01@\0\0\x02\x04\0\x10ins\
tance-network\x01\x03\x03\0#wasi:sockets/instance-network@0.2.6\x05\x1e\x02\x03\0\
\x13\x0aerror-code\x02\x03\0\x13\x11ip-socket-address\x02\x03\0\x13\x11ip-addres\
s-family\x01BD\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\0\x02\x03\x02\x01\x1d\
\x04\0\x07network\x03\0\x02\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x04\x02\
\x03\x02\x01\x20\x04\0\x11ip-socket-address\x03\0\x06\x02\x03\x02\x01!\x04\0\x11\
ip-address-family\x03\0\x08\x01p}\x01r\x02\x04data\x0a\x0eremote-address\x07\x04\
\0\x11incoming-datagram\x03\0\x0b\x01k\x07\x01r\x02\x04data\x0a\x0eremote-addres\
s\x0d\x04\0\x11outgoing-datagram\x03\0\x0e\x04\0\x0audp-socket\x03\x01\x04\0\x18\
incoming-datagram-stream\x03\x01\x04\0\x18outgoing-datagram-stream\x03\x01\x01h\x10\
\x01h\x03\x01j\0\x01\x05\x01@\x03\x04self\x13\x07network\x14\x0dlocal-address\x07\
\0\x15\x04\0\x1d[method]udp-socket.start-bind\x01\x16\x01@\x01\x04self\x13\0\x15\
\x04\0\x1e[method]udp-socket.finish-bind\x01\x17\x01i\x11\x01i\x12\x01o\x02\x18\x19\
\x01j\x01\x1a\x01\x05\x01@\x02\x04self\x13\x0eremote-address\x0d\0\x1b\x04\0\x19\
[method]udp-socket.stream\x01\x1c\x01j\x01\x07\x01\x05\x01@\x01\x04self\x13\0\x1d\
\x04\0\x20[method]udp-socket.local-address\x01\x1e\x04\0![method]udp-socket.remo\
te-address\x01\x1e\x01@\x01\x04self\x13\0\x09\x04\0![method]udp-socket.address-f\
amily\x01\x1f\x01j\x01}\x01\x05\x01@\x01\x04self\x13\0\x20\x04\0$[method]udp-soc\
ket.unicast-hop-limit\x01!\x01@\x02\x04self\x13\x05value}\0\x15\x04\0([method]ud\
p-socket.set-unicast-hop-limit\x01\"\x01j\x01w\x01\x05\x01@\x01\x04self\x13\0#\x04\
\0&[method]udp-socket.receive-buffer-size\x01$\x01@\x02\x04self\x13\x05valuew\0\x15\
\x04\0*[method]udp-socket.set-receive-buffer-size\x01%\x04\0#[method]udp-socket.\
send-buffer-size\x01$\x04\0'[method]udp-socket.set-send-buffer-size\x01%\x01i\x01\
\x01@\x01\x04self\x13\0&\x04\0\x1c[method]udp-socket.subscribe\x01'\x01h\x11\x01\
p\x0c\x01j\x01)\x01\x05\x01@\x02\x04self(\x0bmax-resultsw\0*\x04\0([method]incom\
ing-datagram-stream.receive\x01+\x01@\x01\x04self(\0&\x04\0*[method]incoming-dat\
agram-stream.subscribe\x01,\x01h\x12\x01@\x01\x04self-\0#\x04\0+[method]outgoing\
-datagram-stream.check-send\x01.\x01p\x0f\x01@\x02\x04self-\x09datagrams/\0#\x04\
\0%[method]outgoing-datagram-stream.send\x010\x01@\x01\x04self-\0&\x04\0*[method\
]outgoing-datagram-stream.subscribe\x011\x03\0\x16wasi:sockets/udp@0.2.6\x05\"\x02\
\x03\0\x15\x0audp-socket\x01B\x0c\x02\x03\x02\x01\x1d\x04\0\x07network\x03\0\0\x02\
\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x02\x02\x03\x02\x01!\x04\0\x11ip-addr\
ess-family\x03\0\x04\x02\x03\x02\x01#\x04\0\x0audp-socket\x03\0\x06\x01i\x07\x01\
j\x01\x08\x01\x03\x01@\x01\x0eaddress-family\x05\0\x09\x04\0\x11create-udp-socke\
t\x01\x0a\x03\0$wasi:sockets/udp-create-socket@0.2.6\x05$\x02\x03\0\x0f\x08durat\
ion\x01BT\x02\x03\x02\x01\x06\x04\0\x0cinput-stream\x03\0\0\x02\x03\x02\x01\x07\x04\
\0\x0doutput-stream\x03\0\x02\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\x04\x02\
\x03\x02\x01%\x04\0\x08duration\x03\0\x06\x02\x03\x02\x01\x1d\x04\0\x07network\x03\
\0\x08\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x0a\x02\x03\x02\x01\x20\x04\
\0\x11ip-socket-address\x03\0\x0c\x02\x03\x02\x01!\x04\0\x11ip-address-family\x03\
\0\x0e\x01m\x03\x07receive\x04send\x04both\x04\0\x0dshutdown-type\x03\0\x10\x04\0\
\x0atcp-socket\x03\x01\x01h\x12\x01h\x09\x01j\0\x01\x0b\x01@\x03\x04self\x13\x07\
network\x14\x0dlocal-address\x0d\0\x15\x04\0\x1d[method]tcp-socket.start-bind\x01\
\x16\x01@\x01\x04self\x13\0\x15\x04\0\x1e[method]tcp-socket.finish-bind\x01\x17\x01\
@\x03\x04self\x13\x07network\x14\x0eremote-address\x0d\0\x15\x04\0\x20[method]tc\
p-socket.start-connect\x01\x18\x01i\x01\x01i\x03\x01o\x02\x19\x1a\x01j\x01\x1b\x01\
\x0b\x01@\x01\x04self\x13\0\x1c\x04\0![method]tcp-socket.finish-connect\x01\x1d\x04\
\0\x1f[method]tcp-socket.start-listen\x01\x17\x04\0\x20[method]tcp-socket.finish\
-listen\x01\x17\x01i\x12\x01o\x03\x1e\x19\x1a\x01j\x01\x1f\x01\x0b\x01@\x01\x04s\
elf\x13\0\x20\x04\0\x19[method]tcp-socket.accept\x01!\x01j\x01\x0d\x01\x0b\x01@\x01\
\x04self\x13\0\"\x04\0\x20[method]tcp-socket.local-address\x01#\x04\0![method]tc\
p-socket.remote-address\x01#\x01@\x01\x04self\x13\0\x7f\x04\0\x1f[method]tcp-soc\
ket.is-listening\x01$\x01@\x01\x04self\x13\0\x0f\x04\0![method]tcp-socket.addres\
s-family\x01%\x01@\x02\x04self\x13\x05valuew\0\x15\x04\0*[method]tcp-socket.set-\
listen-backlog-size\x01&\x01j\x01\x7f\x01\x0b\x01@\x01\x04self\x13\0'\x04\0%[met\
hod]tcp-socket.keep-alive-enabled\x01(\x01@\x02\x04self\x13\x05value\x7f\0\x15\x04\
\0)[method]tcp-socket.set-keep-alive-enabled\x01)\x01j\x01\x07\x01\x0b\x01@\x01\x04\
self\x13\0*\x04\0'[method]tcp-socket.keep-alive-idle-time\x01+\x01@\x02\x04self\x13\
\x05value\x07\0\x15\x04\0+[method]tcp-socket.set-keep-alive-idle-time\x01,\x04\0\
&[method]tcp-socket.keep-alive-interval\x01+\x04\0*[method]tcp-socket.set-keep-a\
live-interval\x01,\x01j\x01y\x01\x0b\x01@\x01\x04self\x13\0-\x04\0#[method]tcp-s\
ocket.keep-alive-count\x01.\x01@\x02\x04self\x13\x05valuey\0\x15\x04\0'[method]t\
cp-socket.set-keep-alive-count\x01/\x01j\x01}\x01\x0b\x01@\x01\x04self\x13\00\x04\
\0\x1c[method]tcp-socket.hop-limit\x011\x01@\x02\x04self\x13\x05value}\0\x15\x04\
\0\x20[method]tcp-socket.set-hop-limit\x012\x01j\x01w\x01\x0b\x01@\x01\x04self\x13\
\03\x04\0&[method]tcp-socket.receive-buffer-size\x014\x04\0*[method]tcp-socket.s\
et-receive-buffer-size\x01&\x04\0#[method]tcp-socket.send-buffer-size\x014\x04\0\
'[method]tcp-socket.set-send-buffer-size\x01&\x01i\x05\x01@\x01\x04self\x13\05\x04\
\0\x1c[method]tcp-socket.subscribe\x016\x01@\x02\x04self\x13\x0dshutdown-type\x11\
\0\x15\x04\0\x1b[method]tcp-socket.shutdown\x017\x03\0\x16wasi:sockets/tcp@0.2.6\
\x05&\x02\x03\0\x17\x0atcp-socket\x01B\x0c\x02\x03\x02\x01\x1d\x04\0\x07network\x03\
\0\0\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x02\x02\x03\x02\x01!\x04\0\x11\
ip-address-family\x03\0\x04\x02\x03\x02\x01'\x04\0\x0atcp-socket\x03\0\x06\x01i\x07\
\x01j\x01\x08\x01\x03\x01@\x01\x0eaddress-family\x05\0\x09\x04\0\x11create-tcp-s\
ocket\x01\x0a\x03\0$wasi:sockets/tcp-create-socket@0.2.6\x05(\x02\x03\0\x13\x0ai\
p-address\x01B\x16\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\0\x02\x03\x02\x01\
\x1d\x04\0\x07network\x03\0\x02\x02\x03\x02\x01\x1f\x04\0\x0aerror-code\x03\0\x04\
\x02\x03\x02\x01)\x04\0\x0aip-address\x03\0\x06\x04\0\x16resolve-address-stream\x03\
\x01\x01h\x08\x01k\x07\x01j\x01\x0a\x01\x05\x01@\x01\x04self\x09\0\x0b\x04\03[me\
thod]resolve-address-stream.resolve-next-address\x01\x0c\x01i\x01\x01@\x01\x04se\
lf\x09\0\x0d\x04\0([method]resolve-address-stream.subscribe\x01\x0e\x01h\x03\x01\
i\x08\x01j\x01\x10\x01\x05\x01@\x02\x07network\x0f\x04names\0\x11\x04\0\x11resol\
ve-addresses\x01\x12\x03\0!wasi:sockets/ip-name-lookup@0.2.6\x05*\x01B\x05\x01p}\
\x01@\x01\x03lenw\0\0\x04\0\x10get-random-bytes\x01\x01\x01@\0\0w\x04\0\x0eget-r\
andom-u64\x01\x02\x03\0\x18wasi:random/random@0.2.6\x05+\x01B\x05\x01p}\x01@\x01\
\x03lenw\0\0\x04\0\x19get-insecure-random-bytes\x01\x01\x01@\0\0w\x04\0\x17get-i\
nsecure-random-u64\x01\x02\x03\0\x1awasi:random/insecure@0.2.6\x05,\x01B\x03\x01\
o\x02ww\x01@\0\0\0\x04\0\x0dinsecure-seed\x01\x01\x03\0\x1fwasi:random/insecure-\
seed@0.2.6\x05-\x04\0Gcatscope:witbot/catscopevalidator-with-all-of-its-exports-\
removed@0.1.0\x04\0\x0b7\x01\01catscopevalidator-with-all-of-its-exports-removed\
\x03\0\0\0G\x09producers\x01\x0cprocessed-by\x02\x0dwit-component\x070.236.1\x10\
wit-bindgen-rust\x060.44.0";
#[inline(never)]
#[doc(hidden)]
pub fn __link_custom_section_describing_imports() {
    wit_bindgen::rt::maybe_link_cabi_realloc();
}
#[derive(Debug)]
pub struct Stub;
