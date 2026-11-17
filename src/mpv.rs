//! Minimal libmpv binding, loaded at runtime from the `libmpv-2.dll` found on PATH.
//!
//! Loading at runtime (instead of linking) means cheapedit can start, explain
//! what is missing, and pick the DLL up as soon as the user installs it.
//! Every raw handle is owned by a wrapper whose `Drop` frees it.

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::Arc;

#[repr(C)]
pub struct MpvHandle {
    _p: [u8; 0],
}
#[repr(C)]
pub struct MpvRenderContext {
    _p: [u8; 0],
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct OpenGlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

#[repr(C)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_DOUBLE: c_int = 5;
const RENDER_PARAM_API_TYPE: c_int = 1;
const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const RENDER_PARAM_OPENGL_FBO: c_int = 3;
const RENDER_PARAM_FLIP_Y: c_int = 4;
const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
const RENDER_UPDATE_FRAME: u64 = 1;
const EVENT_NONE: c_int = 0;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_VIDEO_RECONFIG: c_int = 17;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;

// Properties the app reads, watched through events (see `Player`).
const OBS_TIME_POS: u64 = 1;
const OBS_PAUSE: u64 = 2;
const OBS_SEEKING: u64 = 3;
const OBS_DURATION: u64 = 4;

type UpdateCb = unsafe extern "C" fn(*mut c_void);

/// Function table loaded from the DLL.
pub struct Api {
    _lib: libloading::Library,
    // `unsigned long` is 32-bit on Windows.
    client_api_version: unsafe extern "C" fn() -> u32,
    create: unsafe extern "C" fn() -> *mut MpvHandle,
    initialize: unsafe extern "C" fn(*mut MpvHandle) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut MpvHandle),
    set_option_string: unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int,
    set_property_async: unsafe extern "C" fn(*mut MpvHandle, u64, *const c_char, c_int, *mut c_void) -> c_int,
    observe_property: unsafe extern "C" fn(*mut MpvHandle, u64, *const c_char, c_int) -> c_int,
    command_async: unsafe extern "C" fn(*mut MpvHandle, u64, *mut *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(*mut MpvHandle, f64) -> *mut MpvEvent,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    render_create: unsafe extern "C" fn(*mut *mut MpvRenderContext, *mut MpvHandle, *mut RenderParam) -> c_int,
    render_set_update_callback: unsafe extern "C" fn(*mut MpvRenderContext, Option<UpdateCb>, *mut c_void),
    render_update: unsafe extern "C" fn(*mut MpvRenderContext) -> u64,
    render_render: unsafe extern "C" fn(*mut MpvRenderContext, *mut RenderParam) -> c_int,
    render_free: unsafe extern "C" fn(*mut MpvRenderContext),
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }
            .map_err(|_| format!("libmpv-2.dll is missing the function {} — it may be too old or not a real libmpv build.", $name))?
    };
}

impl Api {
    /// Load `libmpv-2.dll` from an explicit path.
    pub fn load(path: &Path) -> Result<Arc<Api>, String> {
        let lib = unsafe { libloading::Library::new(path) }.map_err(|e| {
            let msg = e.to_string();
            if msg.contains("193") || msg.to_lowercase().contains("not a valid win32") {
                "This libmpv-2.dll is 32-bit. cheapedit needs the 64-bit (x86_64) build.".to_string()
            } else {
                format!("Windows couldn't load libmpv-2.dll ({msg}).")
            }
        })?;
        let api = Api {
            client_api_version: sym!(lib, "mpv_client_api_version"),
            create: sym!(lib, "mpv_create"),
            initialize: sym!(lib, "mpv_initialize"),
            terminate_destroy: sym!(lib, "mpv_terminate_destroy"),
            set_option_string: sym!(lib, "mpv_set_option_string"),
            set_property_async: sym!(lib, "mpv_set_property_async"),
            observe_property: sym!(lib, "mpv_observe_property"),
            command_async: sym!(lib, "mpv_command_async"),
            wait_event: sym!(lib, "mpv_wait_event"),
            error_string: sym!(lib, "mpv_error_string"),
            render_create: sym!(lib, "mpv_render_context_create"),
            render_set_update_callback: sym!(lib, "mpv_render_context_set_update_callback"),
            render_update: sym!(lib, "mpv_render_context_update"),
            render_render: sym!(lib, "mpv_render_context_render"),
            render_free: sym!(lib, "mpv_render_context_free"),
            _lib: lib,
        };
        Ok(Arc::new(api))
    }

    /// libmpv client API version as (major, minor).
    pub fn version(&self) -> (u32, u32) {
        let v = unsafe { (self.client_api_version)() };
        (v >> 16, v & 0xffff)
    }

    fn err(&self, code: c_int) -> String {
        unsafe { CStr::from_ptr((self.error_string)(code)) }.to_string_lossy().into_owned()
    }
}

/// Things the player tells the app about since the last poll.
#[derive(Default)]
pub struct PlayerEvents {
    pub file_loaded: bool,
    pub end_file: bool,
    /// The first frame after a load or seek is ready (or the video changed).
    pub restarted: bool,
}

/// One mpv instance. Dropping it destroys the instance.
///
/// Nothing here waits on mpv's core. The UI thread is also mpv's render
/// thread, and the core sometimes waits for a frame to be rendered: a
/// synchronous call made then stalls both until mpv gives up (~200 ms),
/// which made scrubbing crawl. So commands and property writes are queued
/// (`_async`), and the properties the app reads are watched and cached as
/// change events arrive in `poll_events`.
pub struct Player {
    api: Arc<Api>,
    handle: *mut MpvHandle,
    time_pos: Cell<Option<f64>>,
    pause: Cell<Option<bool>>,
    seeking: Cell<Option<bool>>,
    duration: Cell<Option<f64>>,
}

impl Player {
    pub fn new(api: Arc<Api>) -> Result<Player, String> {
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("libmpv couldn't create a player.".into());
        }
        let p = Player { api, handle, time_pos: Cell::new(None), pause: Cell::new(None), seeking: Cell::new(None), duration: Cell::new(None) };
        for (k, v) in [
            ("vo", "libmpv"),
            ("hwdec", "auto-copy"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("pause", "yes"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("terminal", "no"),
            ("hr-seek", "yes"),
            ("audio-display", "no"),
            ("volume-max", "200"),
            ("config", "no"),
        ] {
            p.set_option(k, v)?;
        }
        let rc = unsafe { (p.api.initialize)(p.handle) };
        if rc < 0 {
            return Err(format!("libmpv failed to start: {}", p.api.err(rc)));
        }
        for (id, name, format) in [
            (OBS_TIME_POS, c"time-pos", FORMAT_DOUBLE),
            (OBS_PAUSE, c"pause", FORMAT_FLAG),
            (OBS_SEEKING, c"seeking", FORMAT_FLAG),
            (OBS_DURATION, c"duration", FORMAT_DOUBLE),
        ] {
            let rc = unsafe { (p.api.observe_property)(p.handle, id, name.as_ptr(), format) };
            if rc < 0 {
                return Err(format!("libmpv couldn't watch {name:?}: {}", p.api.err(rc)));
            }
        }
        Ok(p)
    }

    fn set_option(&self, k: &str, v: &str) -> Result<(), String> {
        let (k, v) = (CString::new(k).unwrap(), CString::new(v).unwrap());
        let rc = unsafe { (self.api.set_option_string)(self.handle, k.as_ptr(), v.as_ptr()) };
        if rc < 0 { Err(format!("mpv option {:?}: {}", k, self.api.err(rc))) } else { Ok(()) }
    }

    /// Queue a property change (mpv copies the value).
    pub fn set_property(&self, k: &str, v: &str) {
        let (k, v) = (CString::new(k).unwrap(), CString::new(v).unwrap());
        let mut ptr = v.as_ptr();
        unsafe { (self.api.set_property_async)(self.handle, 0, k.as_ptr(), FORMAT_STRING, &mut ptr as *mut *const c_char as *mut c_void) };
    }

    /// Queue a command. Commands run in the order they're sent.
    pub fn command(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap()).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        unsafe { (self.api.command_async)(self.handle, 0, ptrs.as_mut_ptr()) };
    }

    /// Playback position in seconds, as of the last `poll_events`.
    pub fn time_pos(&self) -> Option<f64> {
        self.time_pos.get()
    }

    /// As of the last `poll_events`.
    pub fn paused(&self) -> Option<bool> {
        self.pause.get()
    }

    /// A seek is still in progress, as of the last `poll_events`.
    pub fn seeking(&self) -> Option<bool> {
        self.seeking.get()
    }

    /// Length of the loaded file in seconds, as of the last `poll_events`.
    pub fn duration(&self) -> Option<f64> {
        self.duration.get()
    }

    pub fn load_file(&self, path: &Path) {
        self.command(&["loadfile", &path.to_string_lossy()]);
    }

    /// Drain mpv's event queue (must be done regularly).
    pub fn poll_events(&self) -> PlayerEvents {
        let mut ev = PlayerEvents::default();
        loop {
            let e = unsafe { &*(self.api.wait_event)(self.handle, 0.0) };
            match e.event_id {
                EVENT_NONE => break,
                EVENT_FILE_LOADED => ev.file_loaded = true,
                EVENT_END_FILE => ev.end_file = true,
                EVENT_VIDEO_RECONFIG | EVENT_PLAYBACK_RESTART => ev.restarted = true,
                EVENT_PROPERTY_CHANGE if !e.data.is_null() => {
                    let prop = unsafe { &*(e.data as *const MpvEventProperty) };
                    // Format 0 (none) means the property is unavailable right now.
                    let num = (prop.format == FORMAT_DOUBLE && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const f64) });
                    let flag = (prop.format == FORMAT_FLAG && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const c_int) } != 0);
                    match e.reply_userdata {
                        OBS_TIME_POS => self.time_pos.set(num),
                        OBS_PAUSE => self.pause.set(flag),
                        OBS_SEEKING => self.seeking.set(flag),
                        OBS_DURATION => self.duration.set(num),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        ev
    }

    /// Create the OpenGL renderer. Must be called with the GL context current.
    /// `on_update` is called from mpv's thread whenever a new frame is ready.
    pub fn create_renderer(&self, on_update: Box<dyn Fn() + Send + Sync>) -> Result<Renderer, String> {
        unsafe extern "C" fn get_proc(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
            unsafe { crate::winutil::gl_proc_address(CStr::from_ptr(name)) }
        }
        unsafe extern "C" fn update_trampoline(data: *mut c_void) {
            let cb = unsafe { &*(data as *const Box<dyn Fn() + Send + Sync>) };
            cb();
        }

        let mut init = OpenGlInitParams { get_proc_address: get_proc, get_proc_address_ctx: std::ptr::null_mut() };
        let api_type = c"opengl";
        let mut params = [
            RenderParam { kind: RENDER_PARAM_API_TYPE, data: api_type.as_ptr() as *mut c_void },
            RenderParam { kind: RENDER_PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
            RenderParam { kind: 0, data: std::ptr::null_mut() },
        ];
        let mut ctx: *mut MpvRenderContext = std::ptr::null_mut();
        let rc = unsafe { (self.api.render_create)(&mut ctx, self.handle, params.as_mut_ptr()) };
        if rc < 0 || ctx.is_null() {
            return Err(format!("libmpv couldn't start its video renderer: {}", self.api.err(rc)));
        }
        let callback = Box::new(on_update);
        let data = &*callback as *const Box<dyn Fn() + Send + Sync> as *mut c_void;
        unsafe { (self.api.render_set_update_callback)(ctx, Some(update_trampoline), data) };
        Ok(Renderer { api: self.api.clone(), ctx, _callback: callback })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe { (self.api.terminate_destroy)(self.handle) };
    }
}

/// mpv's OpenGL renderer. Must be dropped before its `Player`, with GL current.
pub struct Renderer {
    api: Arc<Api>,
    ctx: *mut MpvRenderContext,
    _callback: Box<Box<dyn Fn() + Send + Sync>>,
}

impl Renderer {
    /// True if mpv has a new frame to draw.
    pub fn wants_frame(&self) -> bool {
        unsafe { (self.api.render_update)(self.ctx) & RENDER_UPDATE_FRAME != 0 }
    }

    /// Draw the current frame into an OpenGL framebuffer object.
    pub fn render(&self, fbo: u32, w: i32, h: i32) {
        let mut target = OpenGlFbo { fbo: fbo as c_int, w, h, internal_format: 0 };
        // No flip: we draw into an offscreen texture that egui samples
        // top-down. FLIP_Y=1 is only for drawing straight to the window.
        let mut flip: c_int = 0;
        let mut block: c_int = 0;
        let mut params = [
            RenderParam { kind: RENDER_PARAM_OPENGL_FBO, data: &mut target as *mut _ as *mut c_void },
            RenderParam { kind: RENDER_PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { kind: RENDER_PARAM_BLOCK_FOR_TARGET_TIME, data: &mut block as *mut _ as *mut c_void },
            RenderParam { kind: 0, data: std::ptr::null_mut() },
        ];
        unsafe { (self.api.render_render)(self.ctx, params.as_mut_ptr()) };
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            (self.api.render_set_update_callback)(self.ctx, None, std::ptr::null_mut());
            (self.api.render_free)(self.ctx);
        }
    }
}
