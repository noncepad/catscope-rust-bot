#[allow(dead_code, clippy::all)]
pub mod catscope {
  pub mod witbot {

    #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
    pub mod transactionprocessor {
      #[used]
      #[doc(hidden)]
      static __FORCE_SECTION_REF: fn() =
      super::super::super::__link_custom_section_describing_imports;
      
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
      impl ::core::fmt::Debug for ErrorCode {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
          match self {
            ErrorCode::Unknown => {
              f.debug_tuple("ErrorCode::Unknown").finish()
            }
            ErrorCode::AccessDenied => {
              f.debug_tuple("ErrorCode::AccessDenied").finish()
            }
            ErrorCode::NotSupported => {
              f.debug_tuple("ErrorCode::NotSupported").finish()
            }
            ErrorCode::InvalidArgument => {
              f.debug_tuple("ErrorCode::InvalidArgument").finish()
            }
            ErrorCode::OutOfMemory => {
              f.debug_tuple("ErrorCode::OutOfMemory").finish()
            }
            ErrorCode::Timeout => {
              f.debug_tuple("ErrorCode::Timeout").finish()
            }
          }
        }
      }

      impl ErrorCode{
        #[doc(hidden)]
        pub unsafe fn _lift(val: u8) -> ErrorCode{
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
      pub fn send(signature: &[u8],txdata: &[u8],) -> wit_bindgen::rt::async_support::FutureReader<Result<u64,ErrorCode>>{
        unsafe {
          let vec0 = signature;
          let ptr0 = vec0.as_ptr().cast::<u8>();
          let len0 = vec0.len();
          let vec1 = txdata;
          let ptr1 = vec1.as_ptr().cast::<u8>();
          let len1 = vec1.len();

          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/transactionprocessor@0.1.0")]
          unsafe extern "C" {
            #[link_name = "send"]
            fn wit_import2(_: *mut u8, _: usize, _: *mut u8, _: usize, ) -> i32;
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import2(_: *mut u8, _: usize, _: *mut u8, _: usize, ) -> i32 { unreachable!() }
          let ret = wit_import2(ptr0.cast_mut(), len0, ptr1.cast_mut(), len1);
          wit_bindgen::rt::async_support::FutureReader::new(ret as u32, &super::super::super::wit_future::vtable0::VTABLE)
        }
      }

    }


    #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
    pub mod shooter {
      #[used]
      #[doc(hidden)]
      static __FORCE_SECTION_REF: fn() =
      super::super::super::__link_custom_section_describing_imports;
      
      use super::super::super::_rt;
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
      impl ErrorCode{
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
      impl ::core::fmt::Debug for ErrorCode{
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
          f.debug_struct("ErrorCode")
          .field("code", &(*self as i32))
          .field("name", &self.name())
          .field("message", &self.message())
          .finish()
        }
      }
      impl ::core::fmt::Display for ErrorCode{
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
          write!(f, "{} (error {})", self.name(), *self as i32)
        }
      }

      impl std::error::Error for ErrorCode {}

      impl ErrorCode{
        #[doc(hidden)]
        pub unsafe fn _lift(val: u8) -> ErrorCode{
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
      pub struct Subscription{
        handle: _rt::Resource<Subscription>,
      }

      impl Subscription{
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
      

      unsafe impl _rt::WasmResource for Subscription{
        #[inline]
        unsafe fn drop(_handle: u32) {
          
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
          unsafe extern "C" {
            #[link_name = "[resource-drop]subscription"]
            fn drop(_: i32, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn drop(_: i32, ) { unreachable!() }
          
          unsafe { drop(_handle as i32); }
        }
      }
      
      impl Subscription {
        #[allow(unused_unsafe, clippy::all)]
        #[allow(async_fn_in_trait)]
        pub fn new() -> Self{
          unsafe {

            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
            unsafe extern "C" {
              #[link_name = "[constructor]subscription"]
              fn wit_import0() -> i32;
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import0() -> i32 { unreachable!() }
            let ret = wit_import0();
            Subscription::from_handle(ret as u32)
          }
        }
      }
      impl Subscription {
        #[allow(unused_unsafe, clippy::all)]
        /// Subscribe to a part of the graph.
        #[allow(async_fn_in_trait)]
        pub fn subscribe(&self,id: u64,filter: u32,depth: u32,) -> wit_bindgen::rt::async_support::FutureReader<u32>{
          unsafe {

            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
            unsafe extern "C" {
              #[link_name = "[method]subscription.subscribe"]
              fn wit_import0(_: i32, _: i64, _: i32, _: i32, ) -> i32;
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import0(_: i32, _: i64, _: i32, _: i32, ) -> i32 { unreachable!() }
            let ret = wit_import0((self).handle() as i32, _rt::as_i64(&id), _rt::as_i32(&filter), _rt::as_i32(&depth));
            wit_bindgen::rt::async_support::FutureReader::new(ret as u32, &super::super::super::wit_future::vtable1::VTABLE)
          }
        }
      }
      impl Subscription {
        #[allow(unused_unsafe, clippy::all)]
        /// Cancel a subscription.
        #[allow(async_fn_in_trait)]
        pub fn cancel(&self,subid: u32,) -> (){
          unsafe {

            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
            unsafe extern "C" {
              #[link_name = "[method]subscription.cancel"]
              fn wit_import0(_: i32, _: i32, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import0(_: i32, _: i32, ) { unreachable!() }
            wit_import0((self).handle() as i32, _rt::as_i32(&subid));
          }
        }
      }
      #[allow(unused_unsafe, clippy::all)]
      #[allow(async_fn_in_trait)]
      pub fn account_by_id(id: u64,) -> wit_bindgen::rt::async_support::FutureReader<Result<_rt::Vec::<u8>,ErrorCode>>{
        unsafe {

          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
          unsafe extern "C" {
            #[link_name = "account-by-id"]
            fn wit_import0(_: i64, ) -> i32;
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import0(_: i64, ) -> i32 { unreachable!() }
          let ret = wit_import0(_rt::as_i64(&id));
          wit_bindgen::rt::async_support::FutureReader::new(ret as u32, &super::super::super::wit_future::vtable2::VTABLE)
        }
      }
      #[allow(unused_unsafe, clippy::all)]
      #[allow(async_fn_in_trait)]
      pub fn account_by_pubkey(pubkey: &[u8],) -> wit_bindgen::rt::async_support::FutureReader<Result<_rt::Vec::<u8>,ErrorCode>>{
        unsafe {
          let vec0 = pubkey;
          let ptr0 = vec0.as_ptr().cast::<u8>();
          let len0 = vec0.len();

          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
          unsafe extern "C" {
            #[link_name = "account-by-pubkey"]
            fn wit_import1(_: *mut u8, _: usize, ) -> i32;
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import1(_: *mut u8, _: usize, ) -> i32 { unreachable!() }
          let ret = wit_import1(ptr0.cast_mut(), len0);
          wit_bindgen::rt::async_support::FutureReader::new(ret as u32, &super::super::super::wit_future::vtable2::VTABLE)
        }
      }
      #[allow(unused_unsafe, clippy::all)]
      #[allow(async_fn_in_trait)]
      pub fn subscribe(id: u64,filter: u32,depth: u32,) -> Result<(Subscription,OutputStream,),ErrorCode>{
        unsafe {

          #[repr(align(4))]
          struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
          let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
          let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
          unsafe extern "C" {
            #[link_name = "subscribe"]
            fn wit_import1(_: i64, _: i32, _: i32, _: *mut u8, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import1(_: i64, _: i32, _: i32, _: *mut u8, ) { unreachable!() }
          wit_import1(_rt::as_i64(&id), _rt::as_i32(&filter), _rt::as_i32(&depth), ptr0);
          let l2 = i32::from(*ptr0.add(0).cast::<u8>());
          let result6 = match l2 {
            0 => {
              let e = {
                let l3 = *ptr0.add(4).cast::<i32>();
                let l4 = *ptr0.add(8).cast::<i32>();

                (Subscription::from_handle(l3 as u32), super::super::super::wasi::io::streams::OutputStream::from_handle(l4 as u32))
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
      pub fn subscribebloomfilter(filter: u64,) -> Result<(Subscription,OutputStream,),ErrorCode>{
        unsafe {

          #[repr(align(4))]
          struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
          let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
          let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
          unsafe extern "C" {
            #[link_name = "subscribebloomfilter"]
            fn wit_import1(_: i64, _: *mut u8, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import1(_: i64, _: *mut u8, ) { unreachable!() }
          wit_import1(_rt::as_i64(&filter), ptr0);
          let l2 = i32::from(*ptr0.add(0).cast::<u8>());
          let result6 = match l2 {
            0 => {
              let e = {
                let l3 = *ptr0.add(4).cast::<i32>();
                let l4 = *ptr0.add(8).cast::<i32>();

                (Subscription::from_handle(l3 as u32), super::super::super::wasi::io::streams::OutputStream::from_handle(l4 as u32))
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
      pub fn slot() -> Result<OutputStream,ErrorCode>{
        unsafe {

          #[repr(align(4))]
          struct RetArea([::core::mem::MaybeUninit::<u8>; 8]);
          let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
          let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
          unsafe extern "C" {
            #[link_name = "slot"]
            fn wit_import1(_: *mut u8, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import1(_: *mut u8, ) { unreachable!() }
          wit_import1(ptr0);
          let l2 = i32::from(*ptr0.add(0).cast::<u8>());
          let result5 = match l2 {
            0 => {
              let e = {
                let l3 = *ptr0.add(4).cast::<i32>();

                super::super::super::wasi::io::streams::OutputStream::from_handle(l3 as u32)
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
      pub fn blockhash() -> Result<OutputStream,ErrorCode>{
        unsafe {

          #[repr(align(4))]
          struct RetArea([::core::mem::MaybeUninit::<u8>; 8]);
          let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 8]);
          let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "catscope:witbot/shooter@0.1.0")]
          unsafe extern "C" {
            #[link_name = "blockhash"]
            fn wit_import1(_: *mut u8, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import1(_: *mut u8, ) { unreachable!() }
          wit_import1(ptr0);
          let l2 = i32::from(*ptr0.add(0).cast::<u8>());
          let result5 = match l2 {
            0 => {
              let e = {
                let l3 = *ptr0.add(4).cast::<i32>();

                super::super::super::wasi::io::streams::OutputStream::from_handle(l3 as u32)
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
#[allow(dead_code, clippy::all)]
pub mod wasi {
  pub mod io {

    #[allow(dead_code, async_fn_in_trait, unused_imports, clippy::all)]
    pub mod error {
      #[used]
      #[doc(hidden)]
      static __FORCE_SECTION_REF: fn() =
      super::super::super::__link_custom_section_describing_imports;
      
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
      /// provide functions to further "downcast" this error into more specific
      /// error information. For example, `error`s returned in streams derived
      /// from filesystem types to be described using the filesystem's own
      /// error-code type, using the function
      /// `wasi:filesystem/types/filesystem-error-code`, which takes a parameter
      /// `borrow<error>` and returns
      /// `option<wasi:filesystem/types/error-code>`.
      ///
      /// The set of functions which can "downcast" an `error` into a more
      /// concrete type is open.

      #[derive(Debug)]
      #[repr(transparent)]
      pub struct Error{
        handle: _rt::Resource<Error>,
      }

      impl Error{
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
      

      unsafe impl _rt::WasmResource for Error{
        #[inline]
        unsafe fn drop(_handle: u32) {
          
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "wasi:io/error@0.2.0")]
          unsafe extern "C" {
            #[link_name = "[resource-drop]error"]
            fn drop(_: i32, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn drop(_: i32, ) { unreachable!() }
          
          unsafe { drop(_handle as i32); }
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
        pub fn to_debug_string(&self,) -> _rt::String{
          unsafe {

            #[cfg_attr(target_pointer_width="64", repr(align(8)))]
            #[cfg_attr(target_pointer_width="32", repr(align(4)))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 2*::core::mem::size_of::<*const u8>()]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 2*::core::mem::size_of::<*const u8>()]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/error@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]error.to-debug-string"]
              fn wit_import1(_: i32, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: *mut u8, ) { unreachable!() }
            wit_import1((self).handle() as i32, ptr0);
            let l2 = *ptr0.add(0).cast::<*mut u8>();
            let l3 = *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<usize>();
            let len4 = l3;
            let bytes4 = _rt::Vec::from_raw_parts(l2.cast(), len4, len4);
            let result5 = _rt::string_lift(bytes4);
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
      static __FORCE_SECTION_REF: fn() =
      super::super::super::__link_custom_section_describing_imports;
      
      use super::super::super::_rt;
      /// `pollable` represents a single I/O event which may be ready, or not.

      #[derive(Debug)]
      #[repr(transparent)]
      pub struct Pollable{
        handle: _rt::Resource<Pollable>,
      }

      impl Pollable{
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
      

      unsafe impl _rt::WasmResource for Pollable{
        #[inline]
        unsafe fn drop(_handle: u32) {
          
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "wasi:io/poll@0.2.0")]
          unsafe extern "C" {
            #[link_name = "[resource-drop]pollable"]
            fn drop(_: i32, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn drop(_: i32, ) { unreachable!() }
          
          unsafe { drop(_handle as i32); }
        }
      }
      
      impl Pollable {
        #[allow(unused_unsafe, clippy::all)]
        /// Return the readiness of a pollable. This function never blocks.
        ///
        /// Returns `true` when the pollable is ready, and `false` otherwise.
        #[allow(async_fn_in_trait)]
        pub fn ready(&self,) -> bool{
          unsafe {

            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/poll@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]pollable.ready"]
              fn wit_import0(_: i32, ) -> i32;
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import0(_: i32, ) -> i32 { unreachable!() }
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
        pub fn block(&self,) -> (){
          unsafe {

            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/poll@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]pollable.block"]
              fn wit_import0(_: i32, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import0(_: i32, ) { unreachable!() }
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
      /// If the list contains more elements than can be indexed with a `u32`
      /// value, this function traps.
      ///
      /// A timeout can be implemented by adding a pollable from the
      /// wasi-clocks API to the list.
      ///
      /// This function does not return a `result`; polling in itself does not
      /// do any I/O so it doesn't fail. If any of the I/O sources identified by
      /// the pollables has an error, it is indicated by marking the source as
      /// being reaedy for I/O.
      #[allow(async_fn_in_trait)]
      pub fn poll(in_: &[&Pollable],) -> _rt::Vec::<u32>{
        unsafe {

          #[cfg_attr(target_pointer_width="64", repr(align(8)))]
          #[cfg_attr(target_pointer_width="32", repr(align(4)))]
          struct RetArea([::core::mem::MaybeUninit::<u8>; 2*::core::mem::size_of::<*const u8>()]);
          let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 2*::core::mem::size_of::<*const u8>()]);
          let vec0 = in_;
          let len0 = vec0.len();
          let layout0 = _rt::alloc::Layout::from_size_align(vec0.len() * 4, 4).unwrap();
          let (result0, _cleanup0) = wit_bindgen::rt::Cleanup::new(layout0);for (i, e) in vec0.into_iter().enumerate() {
            let base = result0.add(i * 4);
            {
              *base.add(0).cast::<i32>() = (e).handle() as i32;
            }
          }
          let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "wasi:io/poll@0.2.0")]
          unsafe extern "C" {
            #[link_name = "poll"]
            fn wit_import2(_: *mut u8, _: usize, _: *mut u8, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn wit_import2(_: *mut u8, _: usize, _: *mut u8, ) { unreachable!() }
          wit_import2(result0, len0, ptr1);
          let l3 = *ptr1.add(0).cast::<*mut u8>();
          let l4 = *ptr1.add(::core::mem::size_of::<*const u8>()).cast::<usize>();
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
      static __FORCE_SECTION_REF: fn() =
      super::super::super::__link_custom_section_describing_imports;
      
      use super::super::super::_rt;
      pub type Error = super::super::super::wasi::io::error::Error;
      pub type Pollable = super::super::super::wasi::io::poll::Pollable;
      /// An error for input-stream and output-stream operations.
      pub enum StreamError {
        /// The last operation (a write or flush) failed before completion.
        ///
        /// More information is available in the `error` payload.
        LastOperationFailed(Error),
        /// The stream is closed: no more input will be accepted by the
        /// stream. A closed output-stream will return this error on all
        /// future operations.
        Closed,
      }
      impl ::core::fmt::Debug for StreamError {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
          match self {
            StreamError::LastOperationFailed(e) => {
              f.debug_tuple("StreamError::LastOperationFailed").field(e).finish()
            }
            StreamError::Closed => {
              f.debug_tuple("StreamError::Closed").finish()
            }
          }
        }
      }
      impl ::core::fmt::Display for StreamError {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
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
      pub struct InputStream{
        handle: _rt::Resource<InputStream>,
      }

      impl InputStream{
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
      

      unsafe impl _rt::WasmResource for InputStream{
        #[inline]
        unsafe fn drop(_handle: u32) {
          
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
          unsafe extern "C" {
            #[link_name = "[resource-drop]input-stream"]
            fn drop(_: i32, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn drop(_: i32, ) { unreachable!() }
          
          unsafe { drop(_handle as i32); }
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

      #[derive(Debug)]
      #[repr(transparent)]
      pub struct OutputStream{
        handle: _rt::Resource<OutputStream>,
      }

      impl OutputStream{
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
      

      unsafe impl _rt::WasmResource for OutputStream{
        #[inline]
        unsafe fn drop(_handle: u32) {
          
          #[cfg(target_arch = "wasm32")]
          #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
          unsafe extern "C" {
            #[link_name = "[resource-drop]output-stream"]
            fn drop(_: i32, );
          }

          #[cfg(not(target_arch = "wasm32"))]
          unsafe extern "C" fn drop(_: i32, ) { unreachable!() }
          
          unsafe { drop(_handle as i32); }
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
        pub fn read(&self,len: u64,) -> Result<_rt::Vec::<u8>,StreamError>{
          unsafe {

            #[cfg_attr(target_pointer_width="64", repr(align(8)))]
            #[cfg_attr(target_pointer_width="32", repr(align(4)))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 3*::core::mem::size_of::<*const u8>()]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 3*::core::mem::size_of::<*const u8>()]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]input-stream.read"]
              fn wit_import1(_: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8, ) { unreachable!() }
            wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
            let l2 = i32::from(*ptr0.add(0).cast::<u8>());
            let result9 = match l2 {
              0 => {
                let e = {
                  let l3 = *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<*mut u8>();
                  let l4 = *ptr0.add(2*::core::mem::size_of::<*const u8>()).cast::<usize>();
                  let len5 = l4;

                  _rt::Vec::from_raw_parts(l3.cast(), len5, len5)
                };
                Ok(e)
              }
              1 => {
                let e = {
                  let l6 = i32::from(*ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>());
                  let v8 = match l6 {
                    0 => {
                      let e8 = {
                        let l7 = *ptr0.add(4+1*::core::mem::size_of::<*const u8>()).cast::<i32>();

                        super::super::super::wasi::io::error::Error::from_handle(l7 as u32)
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
        pub fn blocking_read(&self,len: u64,) -> Result<_rt::Vec::<u8>,StreamError>{
          unsafe {

            #[cfg_attr(target_pointer_width="64", repr(align(8)))]
            #[cfg_attr(target_pointer_width="32", repr(align(4)))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 3*::core::mem::size_of::<*const u8>()]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 3*::core::mem::size_of::<*const u8>()]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]input-stream.blocking-read"]
              fn wit_import1(_: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8, ) { unreachable!() }
            wit_import1((self).handle() as i32, _rt::as_i64(&len), ptr0);
            let l2 = i32::from(*ptr0.add(0).cast::<u8>());
            let result9 = match l2 {
              0 => {
                let e = {
                  let l3 = *ptr0.add(::core::mem::size_of::<*const u8>()).cast::<*mut u8>();
                  let l4 = *ptr0.add(2*::core::mem::size_of::<*const u8>()).cast::<usize>();
                  let len5 = l4;

                  _rt::Vec::from_raw_parts(l3.cast(), len5, len5)
                };
                Ok(e)
              }
              1 => {
                let e = {
                  let l6 = i32::from(*ptr0.add(::core::mem::size_of::<*const u8>()).cast::<u8>());
                  let v8 = match l6 {
                    0 => {
                      let e8 = {
                        let l7 = *ptr0.add(4+1*::core::mem::size_of::<*const u8>()).cast::<i32>();

                        super::super::super::wasi::io::error::Error::from_handle(l7 as u32)
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
        pub fn skip(&self,len: u64,) -> Result<u64,StreamError>{
          unsafe {

            #[repr(align(8))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 16]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]input-stream.skip"]
              fn wit_import1(_: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l5 as u32)
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
        pub fn blocking_skip(&self,len: u64,) -> Result<u64,StreamError>{
          unsafe {

            #[repr(align(8))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 16]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]input-stream.blocking-skip"]
              fn wit_import1(_: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l5 as u32)
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
        pub fn subscribe(&self,) -> Pollable{
          unsafe {

            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]input-stream.subscribe"]
              fn wit_import0(_: i32, ) -> i32;
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import0(_: i32, ) -> i32 { unreachable!() }
            let ret = wit_import0((self).handle() as i32);
            super::super::super::wasi::io::poll::Pollable::from_handle(ret as u32)
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
        pub fn check_write(&self,) -> Result<u64,StreamError>{
          unsafe {

            #[repr(align(8))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 16]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.check-write"]
              fn wit_import1(_: i32, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l5 as u32)
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
        pub fn write(&self,contents: &[u8],) -> Result<(),StreamError>{
          unsafe {

            #[repr(align(4))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
            let vec0 = contents;
            let ptr0 = vec0.as_ptr().cast::<u8>();
            let len0 = vec0.len();
            let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.write"]
              fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l5 as u32)
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
        /// // Wait for the stream to become writable
        /// pollable.block();
        /// let Ok(n) = this.check-write(); // eliding error handling
        /// let len = min(n, contents.len());
        /// let (chunk, rest) = contents.split_at(len);
        /// this.write(chunk  );            // eliding error handling
        /// contents = rest;
        /// }
        /// this.flush();
        /// // Wait for completion of `flush`
        /// pollable.block();
        /// // Check for any errors that arose during `flush`
        /// let _ = this.check-write();         // eliding error handling
        /// ```
        #[allow(async_fn_in_trait)]
        pub fn blocking_write_and_flush(&self,contents: &[u8],) -> Result<(),StreamError>{
          unsafe {

            #[repr(align(4))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
            let vec0 = contents;
            let ptr0 = vec0.as_ptr().cast::<u8>();
            let len0 = vec0.len();
            let ptr1 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.blocking-write-and-flush"]
              fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import2(_: i32, _: *mut u8, _: usize, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l5 as u32)
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
        pub fn flush(&self,) -> Result<(),StreamError>{
          unsafe {

            #[repr(align(4))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.flush"]
              fn wit_import1(_: i32, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l4 as u32)
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
        pub fn blocking_flush(&self,) -> Result<(),StreamError>{
          unsafe {

            #[repr(align(4))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.blocking-flush"]
              fn wit_import1(_: i32, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l4 as u32)
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
        /// is ready for more writing, or an error has occured. When this
        /// pollable is ready, `check-write` will return `ok(n)` with n>0, or an
        /// error.
        ///
        /// If the stream is closed, this pollable is always ready immediately.
        ///
        /// The created `pollable` is a child resource of the `output-stream`.
        /// Implementations may trap if the `output-stream` is dropped before
        /// all derived `pollable`s created with this function are dropped.
        #[allow(async_fn_in_trait)]
        pub fn subscribe(&self,) -> Pollable{
          unsafe {

            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.subscribe"]
              fn wit_import0(_: i32, ) -> i32;
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import0(_: i32, ) -> i32 { unreachable!() }
            let ret = wit_import0((self).handle() as i32);
            super::super::super::wasi::io::poll::Pollable::from_handle(ret as u32)
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
        pub fn write_zeroes(&self,len: u64,) -> Result<(),StreamError>{
          unsafe {

            #[repr(align(4))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.write-zeroes"]
              fn wit_import1(_: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l4 as u32)
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
        /// // Wait for the stream to become writable
        /// pollable.block();
        /// let Ok(n) = this.check-write(); // eliding error handling
        /// let len = min(n, num_zeroes);
        /// this.write-zeroes(len);         // eliding error handling
        /// num_zeroes -= len;
        /// }
        /// this.flush();
        /// // Wait for completion of `flush`
        /// pollable.block();
        /// // Check for any errors that arose during `flush`
        /// let _ = this.check-write();         // eliding error handling
        /// ```
        #[allow(async_fn_in_trait)]
        pub fn blocking_write_zeroes_and_flush(&self,len: u64,) -> Result<(),StreamError>{
          unsafe {

            #[repr(align(4))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 12]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 12]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.blocking-write-zeroes-and-flush"]
              fn wit_import1(_: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i64, _: *mut u8, ) { unreachable!() }
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

                        super::super::super::wasi::io::error::Error::from_handle(l4 as u32)
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
        /// The behavior of splice is equivelant to:
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
        pub fn splice(&self,src: &InputStream,len: u64,) -> Result<u64,StreamError>{
          unsafe {

            #[repr(align(8))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 16]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.splice"]
              fn wit_import1(_: i32, _: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i32, _: i64, _: *mut u8, ) { unreachable!() }
            wit_import1((self).handle() as i32, (src).handle() as i32, _rt::as_i64(&len), ptr0);
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

                        super::super::super::wasi::io::error::Error::from_handle(l5 as u32)
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
        pub fn blocking_splice(&self,src: &InputStream,len: u64,) -> Result<u64,StreamError>{
          unsafe {

            #[repr(align(8))]
            struct RetArea([::core::mem::MaybeUninit::<u8>; 16]);
            let mut ret_area = RetArea([::core::mem::MaybeUninit::uninit(); 16]);
            let ptr0 = ret_area.0.as_mut_ptr().cast::<u8>();
            #[cfg(target_arch = "wasm32")]
            #[link(wasm_import_module = "wasi:io/streams@0.2.0")]
            unsafe extern "C" {
              #[link_name = "[method]output-stream.blocking-splice"]
              fn wit_import1(_: i32, _: i32, _: i64, _: *mut u8, );
            }

            #[cfg(not(target_arch = "wasm32"))]
            unsafe extern "C" fn wit_import1(_: i32, _: i32, _: i64, _: *mut u8, ) { unreachable!() }
            wit_import1((self).handle() as i32, (src).handle() as i32, _rt::as_i64(&len), ptr0);
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

                        super::super::super::wasi::io::error::Error::from_handle(l5 as u32)
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
}
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
    // NB: This would ideally be `u32` but it is not. The fact that this has
    // interior mutability is not exposed in the API of this type except for the
    // `take_handle` method which is supposed to in theory be private.
    //
    // This represents, almost all the time, a valid handle value. When it's
    // invalid it's stored as `u32::MAX`.
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
      f.debug_struct("Resource")
      .field("handle", &self.handle)
      .finish()
    }
  }

  impl<T: WasmResource> Drop for Resource<T> {
    fn drop(&mut self) {
      unsafe {
        match self.handle.load(Relaxed) {
          // If this handle was "taken" then don't do anything in the
          // destructor.
          u32::MAX => {}

          // ... but otherwise do actually destroy it with the imported
          // component model intrinsic as defined through `T`.
          other => T::drop(other),
        }
      }
    }
  }
  pub use alloc_crate::string::String;
  pub use alloc_crate::vec::Vec;
  pub unsafe fn string_lift(bytes: Vec<u8>) -> String {
    if cfg!(debug_assertions) {
      String::from_utf8(bytes).unwrap()
    } else {
      unsafe { String::from_utf8_unchecked(bytes) }
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
  pub unsafe fn cabi_dealloc(ptr: *mut u8, size: usize, align: usize) {
    if size == 0 {
      return;
    }
    unsafe {
      let layout = alloc::Layout::from_size_align_unchecked(size, align);
      alloc::dealloc(ptr, layout);
    }
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
    unsafe extern "C" fn cancel_write(_: u32) -> u32 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn cancel_read(_: u32) -> u32 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn drop_writable(_: u32) { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn drop_readable(_: u32) { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn new() -> u64 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn start_read(_: u32, _: *mut u8) -> u32 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn start_write(_: u32, _: *const u8) -> u32 { unreachable!() }

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

    unsafe fn lift(ptr: *mut u8) -> Result<u64,super::super::catscope::witbot::transactionprocessor::ErrorCode> { unsafe { let l0 = i32::from(*ptr.add(0).cast::<u8>());

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

          super::super::catscope::witbot::transactionprocessor::ErrorCode::_lift(l2 as u8)
        };
        Err(e)
      }
      _ => super::super::_rt::invalid_enum_discriminant(),
    } } }
    unsafe fn lower(value: Result<u64,super::super::catscope::witbot::transactionprocessor::ErrorCode>, ptr: *mut u8) { unsafe { match value {
      Ok(e) => { {
        *ptr.add(0).cast::<u8>() = (0i32) as u8;
        *ptr.add(8).cast::<i64>() = super::super::_rt::as_i64(e);
      } },
      Err(e) => { {
        *ptr.add(0).cast::<u8>() = (1i32) as u8;
        *ptr.add(8).cast::<u8>() = (e.clone() as i32) as u8;
      } },
    }; } }
    unsafe fn dealloc_lists(ptr: *mut u8) { unsafe {  } }

    pub static VTABLE: wit_bindgen::rt::async_support::FutureVtable<Result<u64,super::super::catscope::witbot::transactionprocessor::ErrorCode>> = wit_bindgen::rt::async_support::FutureVtable::<Result<u64,super::super::catscope::witbot::transactionprocessor::ErrorCode>> {
      cancel_write,
      cancel_read,
      drop_writable,
      drop_readable,
      dealloc_lists,
      layout: unsafe {
        ::std::alloc::Layout::from_size_align_unchecked(16, 8)
      },
      lift,
      lower,
      new,
      start_read,
      start_write,
    };

    impl super::FuturePayload for Result<u64,super::super::catscope::witbot::transactionprocessor::ErrorCode> {
      const VTABLE: &'static wit_bindgen::rt::async_support::FutureVtable<Self> = &VTABLE;
    }
  }
  
  #[doc(hidden)]
  #[allow(unused_unsafe)]
  pub mod vtable1 {

    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn cancel_write(_: u32) -> u32 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn cancel_read(_: u32) -> u32 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn drop_writable(_: u32) { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn drop_readable(_: u32) { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn new() -> u64 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn start_read(_: u32, _: *mut u8) -> u32 { unreachable!() }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn start_write(_: u32, _: *const u8) -> u32 { unreachable!() }

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

    unsafe fn lift(ptr: *mut u8) -> u32 { unsafe { let l0 = *ptr.add(0).cast::<i32>();

    l0 as u32 } }
    unsafe fn lower(value: u32, ptr: *mut u8) { unsafe { *ptr.add(0).cast::<i32>() = super::super::_rt::as_i32(value);
  } }
  unsafe fn dealloc_lists(ptr: *mut u8) { unsafe {  } }

  pub static VTABLE: wit_bindgen::rt::async_support::FutureVtable<u32> = wit_bindgen::rt::async_support::FutureVtable::<u32> {
    cancel_write,
    cancel_read,
    drop_writable,
    drop_readable,
    dealloc_lists,
    layout: unsafe {
      ::std::alloc::Layout::from_size_align_unchecked(4, 4)
    },
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
  unsafe extern "C" fn cancel_write(_: u32) -> u32 { unreachable!() }
  #[cfg(not(target_arch = "wasm32"))]
  unsafe extern "C" fn cancel_read(_: u32) -> u32 { unreachable!() }
  #[cfg(not(target_arch = "wasm32"))]
  unsafe extern "C" fn drop_writable(_: u32) { unreachable!() }
  #[cfg(not(target_arch = "wasm32"))]
  unsafe extern "C" fn drop_readable(_: u32) { unreachable!() }
  #[cfg(not(target_arch = "wasm32"))]
  unsafe extern "C" fn new() -> u64 { unreachable!() }
  #[cfg(not(target_arch = "wasm32"))]
  unsafe extern "C" fn start_read(_: u32, _: *mut u8) -> u32 { unreachable!() }
  #[cfg(not(target_arch = "wasm32"))]
  unsafe extern "C" fn start_write(_: u32, _: *const u8) -> u32 { unreachable!() }

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

  unsafe fn lift(ptr: *mut u8) -> Result<super::super::_rt::Vec::<u8>,super::super::catscope::witbot::shooter::ErrorCode> { unsafe { let l0 = i32::from(*ptr.add(0).cast::<u8>());

  match l0 {
    0 => {
      let e = {
        let l1 = *ptr.add(::core::mem::size_of::<*const u8>()).cast::<*mut u8>();
        let l2 = *ptr.add(2*::core::mem::size_of::<*const u8>()).cast::<usize>();
        let len3 = l2;

        super::super::_rt::Vec::from_raw_parts(l1.cast(), len3, len3)
      };
      Ok(e)
    }
    1 => {
      let e = {
        let l4 = i32::from(*ptr.add(::core::mem::size_of::<*const u8>()).cast::<u8>());

        super::super::catscope::witbot::shooter::ErrorCode::_lift(l4 as u8)
      };
      Err(e)
    }
    _ => super::super::_rt::invalid_enum_discriminant(),
  } } }
  unsafe fn lower(value: Result<super::super::_rt::Vec::<u8>,super::super::catscope::witbot::shooter::ErrorCode>, ptr: *mut u8) { unsafe { match value {
    Ok(e) => { {
      *ptr.add(0).cast::<u8>() = (0i32) as u8;
      let vec0 = (e).into_boxed_slice();
      let ptr0 = vec0.as_ptr().cast::<u8>();
      let len0 = vec0.len();
      ::core::mem::forget(vec0);
      *ptr.add(2*::core::mem::size_of::<*const u8>()).cast::<usize>() = len0;
      *ptr.add(::core::mem::size_of::<*const u8>()).cast::<*mut u8>() = ptr0.cast_mut();
    } },
    Err(e) => { {
      *ptr.add(0).cast::<u8>() = (1i32) as u8;
      *ptr.add(::core::mem::size_of::<*const u8>()).cast::<u8>() = (e.clone() as i32) as u8;
    } },
  }; } }
  unsafe fn dealloc_lists(ptr: *mut u8) { unsafe { let l0 = i32::from(*ptr.add(0).cast::<u8>());
  match l0 {
    0 => {
      let l1 = *ptr.add(::core::mem::size_of::<*const u8>()).cast::<*mut u8>();
      let l2 = *ptr.add(2*::core::mem::size_of::<*const u8>()).cast::<usize>();
      let base3 = l1;
      let len3 = l2;
      super::super::_rt::cabi_dealloc(base3, len3 * 1, 1);
    },
    _ => (),
  }
} }

pub static VTABLE: wit_bindgen::rt::async_support::FutureVtable<Result<super::super::_rt::Vec::<u8>,super::super::catscope::witbot::shooter::ErrorCode>> = wit_bindgen::rt::async_support::FutureVtable::<Result<super::super::_rt::Vec::<u8>,super::super::catscope::witbot::shooter::ErrorCode>> {
  cancel_write,
  cancel_read,
  drop_writable,
  drop_readable,
  dealloc_lists,
  layout: unsafe {
    ::std::alloc::Layout::from_size_align_unchecked(12, 4)
  },
  lift,
  lower,
  new,
  start_read,
  start_write,
};

impl super::FuturePayload for Result<super::super::_rt::Vec::<u8>,super::super::catscope::witbot::shooter::ErrorCode> {
  const VTABLE: &'static wit_bindgen::rt::async_support::FutureVtable<Self> = &VTABLE;
}
}
/// Creates a new Component Model `future` with the specified payload type.
///
/// The `default` function provided computes the default value to be sent in
/// this future if no other value was otherwise sent.
pub fn new<T: FuturePayload>(default: fn() -> T) -> (wit_bindgen::rt::async_support::FutureWriter<T>, wit_bindgen::rt::async_support::FutureReader<T>) {
  unsafe { wit_bindgen::rt::async_support::future_new::<T>(default, T::VTABLE) }
}
}

#[cfg(target_arch = "wasm32")]
#[unsafe(link_section = "component-type:wit-bindgen:0.44.0:catscope:witbot@0.1.0:catscopevalidator:encoded world")]
#[doc(hidden)]
#[allow(clippy::octal_escapes)]
pub static __WIT_BINDGEN_COMPONENT_TYPE: [u8; 2110] = *b"\
\0asm\x0d\0\x01\0\0\x19\x16wit-component-encoding\x04\0\x07\xb6\x0f\x01A\x02\x01\
A\x0e\x01B\x07\x01m\x06\x07unknown\x0daccess-denied\x0dnot-supported\x10invalid-\
argument\x0dout-of-memory\x07timeout\x04\0\x0aerror-code\x03\0\0\x01p}\x01j\x01w\
\x01\x01\x01e\x01\x03\x01@\x02\x09signature\x02\x06txdata\x02\0\x04\x04\0\x04sen\
d\x01\x05\x03\0*catscope:witbot/transactionprocessor@0.1.0\x05\0\x01B\x04\x04\0\x05\
error\x03\x01\x01h\0\x01@\x01\x04self\x01\0s\x04\0\x1d[method]error.to-debug-str\
ing\x01\x02\x03\0\x13wasi:io/error@0.2.0\x05\x01\x01B\x0a\x04\0\x08pollable\x03\x01\
\x01h\0\x01@\x01\x04self\x01\0\x7f\x04\0\x16[method]pollable.ready\x01\x02\x01@\x01\
\x04self\x01\x01\0\x04\0\x16[method]pollable.block\x01\x03\x01p\x01\x01py\x01@\x01\
\x02in\x04\0\x05\x04\0\x04poll\x01\x06\x03\0\x12wasi:io/poll@0.2.0\x05\x02\x02\x03\
\0\x01\x05error\x02\x03\0\x02\x08pollable\x01B(\x02\x03\x02\x01\x03\x04\0\x05err\
or\x03\0\0\x02\x03\x02\x01\x04\x04\0\x08pollable\x03\0\x02\x01i\x01\x01q\x02\x15\
last-operation-failed\x01\x04\0\x06closed\0\0\x04\0\x0cstream-error\x03\0\x05\x04\
\0\x0cinput-stream\x03\x01\x04\0\x0doutput-stream\x03\x01\x01h\x07\x01p}\x01j\x01\
\x0a\x01\x06\x01@\x02\x04self\x09\x03lenw\0\x0b\x04\0\x19[method]input-stream.re\
ad\x01\x0c\x04\0\"[method]input-stream.blocking-read\x01\x0c\x01j\x01w\x01\x06\x01\
@\x02\x04self\x09\x03lenw\0\x0d\x04\0\x19[method]input-stream.skip\x01\x0e\x04\0\
\"[method]input-stream.blocking-skip\x01\x0e\x01i\x03\x01@\x01\x04self\x09\0\x0f\
\x04\0\x1e[method]input-stream.subscribe\x01\x10\x01h\x08\x01@\x01\x04self\x11\0\
\x0d\x04\0![method]output-stream.check-write\x01\x12\x01j\0\x01\x06\x01@\x02\x04\
self\x11\x08contents\x0a\0\x13\x04\0\x1b[method]output-stream.write\x01\x14\x04\0\
.[method]output-stream.blocking-write-and-flush\x01\x14\x01@\x01\x04self\x11\0\x13\
\x04\0\x1b[method]output-stream.flush\x01\x15\x04\0$[method]output-stream.blocki\
ng-flush\x01\x15\x01@\x01\x04self\x11\0\x0f\x04\0\x1f[method]output-stream.subsc\
ribe\x01\x16\x01@\x02\x04self\x11\x03lenw\0\x13\x04\0\"[method]output-stream.wri\
te-zeroes\x01\x17\x04\05[method]output-stream.blocking-write-zeroes-and-flush\x01\
\x17\x01@\x03\x04self\x11\x03src\x09\x03lenw\0\x0d\x04\0\x1c[method]output-strea\
m.splice\x01\x18\x04\0%[method]output-stream.blocking-splice\x01\x18\x03\0\x15wa\
si:io/streams@0.2.0\x05\x05\x02\x03\0\x03\x0cinput-stream\x02\x03\0\x03\x0doutpu\
t-stream\x01B\"\x02\x03\x02\x01\x06\x04\0\x0cinput-stream\x03\0\0\x02\x03\x02\x01\
\x07\x04\0\x0doutput-stream\x03\0\x02\x01m\x06\x07unknown\x0daccess-denied\x0dno\
t-supported\x10invalid-argument\x0dout-of-memory\x07timeout\x04\0\x0aerror-code\x03\
\0\x04\x04\0\x0csubscription\x03\x01\x01i\x06\x01@\0\0\x07\x04\0\x19[constructor\
]subscription\x01\x08\x01h\x06\x01e\x01y\x01@\x04\x04self\x09\x02idw\x06filtery\x05\
depthy\0\x0a\x04\0\x1e[method]subscription.subscribe\x01\x0b\x01@\x02\x04self\x09\
\x05subidy\x01\0\x04\0\x1b[method]subscription.cancel\x01\x0c\x01p}\x01j\x01\x0d\
\x01\x05\x01e\x01\x0e\x01@\x01\x02idw\0\x0f\x04\0\x0daccount-by-id\x01\x10\x01@\x01\
\x06pubkey\x0d\0\x0f\x04\0\x11account-by-pubkey\x01\x11\x01i\x03\x01o\x02\x07\x12\
\x01j\x01\x13\x01\x05\x01@\x03\x02idw\x06filtery\x05depthy\0\x14\x04\0\x09subscr\
ibe\x01\x15\x01@\x01\x06filterw\0\x14\x04\0\x14subscribebloomfilter\x01\x16\x01j\
\x01\x12\x01\x05\x01@\0\0\x17\x04\0\x04slot\x01\x18\x04\0\x09blockhash\x01\x18\x03\
\0\x1dcatscope:witbot/shooter@0.1.0\x05\x08\x04\0'catscope:witbot/catscopevalida\
tor@0.1.0\x04\0\x0b\x17\x01\0\x11catscopevalidator\x03\0\0\0G\x09producers\x01\x0c\
processed-by\x02\x0dwit-component\x070.236.1\x10wit-bindgen-rust\x060.44.0";

#[inline(never)]
#[doc(hidden)]
pub fn __link_custom_section_describing_imports() {
  wit_bindgen::rt::maybe_link_cabi_realloc();
}

