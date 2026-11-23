use std::ffi::{c_char, c_void, CString};
use std::ptr;

use libloading::Library;
use slot_gfx::{GfxError, Surface, OUT_H, OUT_W};

type Handle = *mut c_void;
type GetProc = unsafe extern "C" fn(*const c_char) -> *const c_void;
type GetPlatformDisplay = unsafe extern "C" fn(u32, Handle, *const i32) -> Handle;
type Initialize = unsafe extern "C" fn(Handle, *mut i32, *mut i32) -> u32;
type BindApi = unsafe extern "C" fn(u32) -> u32;
type ChooseConfig = unsafe extern "C" fn(Handle, *const i32, *mut Handle, i32, *mut i32) -> u32;
type CreatePbuffer = unsafe extern "C" fn(Handle, Handle, *const i32) -> Handle;
type CreateContext = unsafe extern "C" fn(Handle, Handle, Handle, *const i32) -> Handle;
type MakeCurrent = unsafe extern "C" fn(Handle, Handle, Handle, Handle) -> u32;
type Destroy = unsafe extern "C" fn(Handle, Handle) -> u32;
type Terminate = unsafe extern "C" fn(Handle) -> u32;
type GetError = unsafe extern "C" fn() -> i32;

struct Egl {
    get_proc: GetProc,
    initialize: Initialize,
    bind_api: BindApi,
    choose_config: ChooseConfig,
    create_pbuffer: CreatePbuffer,
    create_context: CreateContext,
    make_current: MakeCurrent,
    destroy_context: Destroy,
    destroy_surface: Destroy,
    terminate: Terminate,
    get_error: GetError,
    _library: Library,
}

impl Egl {
    fn new() -> Result<Self, GfxError> {
        let library = unsafe { Library::new("libEGL.so.1") }
            .map_err(|e| GfxError::Context(format!("libEGL: {e}")))?;
        unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, GfxError> {
            library
                .get::<T>(name)
                .map(|symbol| *symbol)
                .map_err(|e| GfxError::Context(format!("EGL symbol: {e}")))
        }
        unsafe {
            Ok(Self {
                get_proc: symbol(&library, b"eglGetProcAddress")?,
                initialize: symbol(&library, b"eglInitialize")?,
                bind_api: symbol(&library, b"eglBindAPI")?,
                choose_config: symbol(&library, b"eglChooseConfig")?,
                create_pbuffer: symbol(&library, b"eglCreatePbufferSurface")?,
                create_context: symbol(&library, b"eglCreateContext")?,
                make_current: symbol(&library, b"eglMakeCurrent")?,
                destroy_context: symbol(&library, b"eglDestroyContext")?,
                destroy_surface: symbol(&library, b"eglDestroySurface")?,
                terminate: symbol(&library, b"eglTerminate")?,
                get_error: symbol(&library, b"eglGetError")?,
                _library: library,
            })
        }
    }

    fn error(&self, what: &str) -> GfxError {
        GfxError::Context(format!("{what}: EGL error 0x{:x}", unsafe {
            (self.get_error)()
        }))
    }
}

pub struct HeadlessSurface {
    egl: Egl,
    gles: Library,
    display: Handle,
    surface: Handle,
    context: Handle,
    initialized: bool,
}

impl HeadlessSurface {
    pub fn new() -> Result<Self, GfxError> {
        let egl = Egl::new()?;
        let gles = unsafe { Library::new("libGLESv2.so.2") }
            .map_err(|e| GfxError::Context(format!("libGLESv2: {e}")))?;
        let mut result = Self {
            egl,
            gles,
            display: ptr::null_mut(),
            surface: ptr::null_mut(),
            context: ptr::null_mut(),
            initialized: false,
        };
        unsafe {
            let name = CString::new("eglGetPlatformDisplayEXT").expect("static EGL symbol");
            let address = (result.egl.get_proc)(name.as_ptr());
            if address.is_null() {
                return Err(GfxError::Context("surfaceless EGL is unavailable".into()));
            }
            let get_display: GetPlatformDisplay = std::mem::transmute(address);
            result.display = get_display(0x31DD, ptr::null_mut(), ptr::null());
            if result.display.is_null()
                || (result.egl.initialize)(result.display, ptr::null_mut(), ptr::null_mut()) == 0
            {
                return Err(result.egl.error("eglInitialize"));
            }
            result.initialized = true;
            if (result.egl.bind_api)(0x30A0) == 0 {
                return Err(result.egl.error("eglBindAPI"));
            }
            let attributes = [
                0x3033, 1, 0x3040, 4, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3038,
            ];
            let mut config = ptr::null_mut();
            let mut count = 0;
            if (result.egl.choose_config)(
                result.display,
                attributes.as_ptr(),
                &mut config,
                1,
                &mut count,
            ) == 0
                || count == 0
            {
                return Err(result.egl.error("eglChooseConfig"));
            }
            let surface_attrs = [0x3057, OUT_W as i32, 0x3056, OUT_H as i32, 0x3038];
            result.surface =
                (result.egl.create_pbuffer)(result.display, config, surface_attrs.as_ptr());
            if result.surface.is_null() {
                return Err(result.egl.error("eglCreatePbufferSurface"));
            }
            let context_attrs = [0x3098, 2, 0x3038];
            result.context = (result.egl.create_context)(
                result.display,
                config,
                ptr::null_mut(),
                context_attrs.as_ptr(),
            );
            if result.context.is_null() {
                return Err(result.egl.error("eglCreateContext"));
            }
        }
        result.make_current()?;
        Ok(result)
    }
}

impl Surface for HeadlessSurface {
    fn make_current(&mut self) -> Result<(), GfxError> {
        if unsafe {
            (self.egl.make_current)(self.display, self.surface, self.surface, self.context)
        } == 0
        {
            Err(self.egl.error("eglMakeCurrent"))
        } else {
            Ok(())
        }
    }

    fn window_size(&self) -> (u32, u32) {
        (OUT_W, OUT_H)
    }

    fn swap(&mut self) -> Result<(), GfxError> {
        Ok(())
    }

    fn proc_address(&self, name: &str) -> *const c_void {
        let Ok(name) = CString::new(name) else {
            return ptr::null();
        };
        let exported = unsafe { self.gles.get::<unsafe extern "C" fn()>(name.as_bytes()) };
        match exported {
            Ok(function) => *function as *const c_void,
            Err(_) => unsafe { (self.egl.get_proc)(name.as_ptr()) },
        }
    }
}

impl Drop for HeadlessSurface {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                (self.egl.make_current)(
                    self.display,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                if !self.context.is_null() {
                    (self.egl.destroy_context)(self.display, self.context);
                }
                if !self.surface.is_null() {
                    (self.egl.destroy_surface)(self.display, self.surface);
                }
                (self.egl.terminate)(self.display);
            }
        }
    }
}
