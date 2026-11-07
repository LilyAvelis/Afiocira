//! libmpv owns decoding, audio and playback. Only this adapter touches its C ABI.
//! The render context is used/freed only on egui's GL thread; async commands and
//! nonblocking events avoid waiting for the renderer from that same thread.
use eframe::{egui, glow};
use glow::HasContext;
use libloading::Library;
use photo_core::api::VideoState;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type Handle = *mut c_void;
type Resolver = Arc<dyn Fn(&CStr) -> *const c_void + Send + Sync>;
#[repr(C)]
struct Param {
    kind: c_int,
    data: *mut c_void,
}
impl Param {
    fn new<T>(kind: c_int, data: &mut T) -> Self {
        Self {
            kind,
            data: (data as *mut T).cast(),
        }
    }
    fn end() -> Self {
        Self {
            kind: 0,
            data: std::ptr::null_mut(),
        }
    }
}
#[repr(C)]
struct GlInit {
    resolver: unsafe extern "C" fn(Handle, *const c_char) -> *mut c_void,
    context: Handle,
}
#[repr(C)]
struct Fbo {
    fbo: c_int,
    width: c_int,
    height: c_int,
    format: c_int,
}
#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: Handle,
}
#[repr(C)]
struct Property {
    name: *const c_char,
    format: c_int,
    data: Handle,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

struct Api {
    create: unsafe extern "C" fn() -> Handle,
    init: unsafe extern "C" fn(Handle) -> c_int,
    option: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    command: unsafe extern "C" fn(Handle, u64, *const *const c_char) -> c_int,
    observe: unsafe extern "C" fn(Handle, u64, *const c_char, c_int) -> c_int,
    event: unsafe extern "C" fn(Handle, f64) -> *const Event,
    error: unsafe extern "C" fn(c_int) -> *const c_char,
    destroy: unsafe extern "C" fn(Handle),
    render_create: unsafe extern "C" fn(*mut Handle, Handle, *mut Param) -> c_int,
    render: unsafe extern "C" fn(Handle, *mut Param) -> c_int,
    render_free: unsafe extern "C" fn(Handle),
    _library: Library,
}
impl Api {
    unsafe fn load(path: &Path) -> Result<Self, String> {
        let library = unsafe { Library::new(path) }.map_err(|e| e.to_string())?;
        // Signatures and repr(C) layouts match include/mpv/{client,render,render_gl}.h.
        macro_rules! sym {
            ($name:literal) => {
                unsafe {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| e.to_string())?
                }
            };
        }
        Ok(Self {
            create: sym!("mpv_create"),
            init: sym!("mpv_initialize"),
            option: sym!("mpv_set_option_string"),
            command: sym!("mpv_command_async"),
            observe: sym!("mpv_observe_property"),
            event: sym!("mpv_wait_event"),
            error: sym!("mpv_error_string"),
            destroy: sym!("mpv_terminate_destroy"),
            render_create: sym!("mpv_render_context_create"),
            render: sym!("mpv_render_context_render"),
            render_free: sym!("mpv_render_context_free"),
            _library: library,
        })
    }
    fn check(&self, code: c_int) -> Result<(), String> {
        if code >= 0 {
            Ok(())
        } else {
            Err(unsafe { CStr::from_ptr((self.error)(code)) }
                .to_string_lossy()
                .into_owned())
        }
    }
}

pub struct Video {
    api: Api,
    handle: Handle,
    render: Handle,
    resolver: Resolver,
    target: Option<(glow::Framebuffer, glow::Texture, [i32; 2])>,
    pub state: VideoState,
}
// The mutex serializes access. GL operations are called only by the render
// callback and on_exit (same GL thread). No raw pointer escapes this module.
unsafe impl Send for Video {}

pub fn runtime_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("AFIO_MPV_DLL") {
        return Ok(path.into());
    }
    let path = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("libmpv-2.dll");
    if path.is_file() {
        Ok(path)
    } else {
        Err("Video engine missing. Run scripts/setup-video.ps1 and restart Afio.".into())
    }
}

impl Video {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Arc<Mutex<Self>>, String> {
        let api = unsafe { Api::load(&runtime_path()?) }?;
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("Cannot create libmpv".into());
        }
        let mut video = Self {
            api,
            handle,
            render: std::ptr::null_mut(),
            resolver: cc.get_proc_address.clone().ok_or("Video requires OpenGL")?,
            target: None,
            state: VideoState::default(),
        };
        for (key, value) in [
            ("vo", "libmpv"),
            ("config", "no"),
            ("terminal", "no"),
            ("input-default-bindings", "no"),
            ("hwdec", "auto-safe"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("video-timing-offset", "0"),
        ] {
            let key = CString::new(key).unwrap();
            let value = CString::new(value).unwrap();
            video
                .api
                .check(unsafe { (video.api.option)(handle, key.as_ptr(), value.as_ptr()) })?;
        }
        video.api.check(unsafe { (video.api.init)(handle) })?;
        for (index, name, format) in [
            (1, "time-pos", 5),
            (2, "duration", 5),
            (3, "pause", 3),
            (4, "volume", 5),
        ] {
            let name = CString::new(name).unwrap();
            video
                .api
                .check(unsafe { (video.api.observe)(handle, index, name.as_ptr(), format) })?;
        }
        video.init_renderer()?;
        video.state.volume = 80.0;
        video.command(&["set", "volume", "80"])?;
        Ok(Arc::new(Mutex::new(video)))
    }
    pub fn command(&mut self, args: &[&str]) -> Result<(), String> {
        let args: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        let mut pointers: Vec<_> = args.iter().map(|s| s.as_ptr()).collect();
        pointers.push(std::ptr::null());
        self.api
            .check(unsafe { (self.api.command)(self.handle, 0, pointers.as_ptr()) })
    }
    pub fn sync_path(&mut self, path: Option<&Path>) {
        if self.state.path.as_deref() == path {
            return;
        }
        self.state.error = None;
        self.state.position = 0.0;
        self.state.duration = 0.0;
        let result = if let Some(path) = path {
            self.command(&["loadfile", &path.to_string_lossy(), "replace"])
                .and_then(|()| self.command(&["set", "pause", "no"]))
        } else {
            self.command(&["stop"])
        };
        self.state.path = path.map(Path::to_path_buf);
        if let Err(error) = result {
            self.state.error = Some(error);
        }
    }
    pub fn poll(&mut self) {
        for _ in 0..128 {
            let event = unsafe { &*(self.api.event)(self.handle, 0.0) };
            if event.id == 0 {
                break;
            }
            if event.error < 0 {
                self.state.error = self.api.check(event.error).err();
            }
            if event.id == 7 && !event.data.is_null() {
                let end = unsafe { &*event.data.cast::<EndFile>() };
                if end.reason == 4 {
                    self.state.error = self.api.check(end.error).err();
                }
            }
            if event.id == 22 && !event.data.is_null() {
                let p = unsafe { &*event.data.cast::<Property>() };
                if p.data.is_null() {
                    continue;
                }
                match (event.userdata, p.format) {
                    (1, 5) => self.state.position = unsafe { *p.data.cast::<f64>() },
                    (2, 5) => self.state.duration = unsafe { *p.data.cast::<f64>() },
                    (3, 3) => self.state.paused = unsafe { *p.data.cast::<c_int>() != 0 },
                    (4, 5) => self.state.volume = unsafe { *p.data.cast::<f64>() },
                    _ => {}
                }
            }
        }
    }
    fn init_renderer(&mut self) -> Result<(), String> {
        unsafe {
            unsafe extern "C" fn resolve(context: Handle, name: *const c_char) -> *mut c_void {
                let resolver = unsafe { &*context.cast::<Resolver>() };
                resolver(unsafe { CStr::from_ptr(name) }).cast_mut()
            }
            let mut init = GlInit {
                resolver: resolve,
                context: (&mut self.resolver as *mut Resolver).cast(),
            };
            let mut params = [
                Param {
                    kind: 1,
                    data: c"opengl".as_ptr().cast_mut().cast(),
                },
                Param::new(2, &mut init),
                Param::end(),
            ];
            self.api.check((self.api.render_create)(
                &mut self.render,
                self.handle,
                params.as_mut_ptr(),
            ))?;
        }
        Ok(())
    }
    unsafe fn paint(
        &mut self,
        info: egui::PaintCallbackInfo,
        painter: &eframe::egui_glow::Painter,
    ) -> Result<(), String> {
        // Safety: this callback runs with eframe's GL context current.
        unsafe {
            let gl = painter.gl();
            let viewport = info.viewport_in_pixels();
            let size = [viewport.width_px.max(1), viewport.height_px.max(1)];
            if self.target.as_ref().is_none_or(|t| t.2 != size) {
                self.free_target(gl);
                let texture = gl.create_texture()?;
                let fbo = gl.create_framebuffer()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    size[0],
                    size[1],
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
                self.target = Some((fbo, texture, size));
                if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                    return Err("Video framebuffer incomplete".into());
                }
            }
            let fbo = self.target.as_ref().unwrap().0;
            let mut target = Fbo {
                fbo: fbo.0.get() as i32,
                width: size[0],
                height: size[1],
                format: glow::RGBA8 as i32,
            };
            let mut block = 0i32;
            // mpv's texture-oriented output is top-down. We blit this FBO
            // directly into the window's bottom-up OpenGL framebuffer, so
            // request the window orientation once (do not mirror the X axis).
            let mut flip_y = 1i32;
            let mut params = [
                Param::new(3, &mut target),
                Param::new(4, &mut flip_y), // MPV_RENDER_PARAM_FLIP_Y
                Param::new(12, &mut block),
                Param::end(),
            ];
            self.api
                .check((self.api.render)(self.render, params.as_mut_ptr()))?;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, painter.intermediate_fbo());
            let clip = info.clip_rect_in_pixels();
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left_px,
                clip.from_bottom_px,
                clip.width_px,
                clip.height_px,
            );
            gl.blit_framebuffer(
                0,
                0,
                size[0],
                size[1],
                viewport.left_px,
                viewport.from_bottom_px,
                viewport.left_px + size[0],
                viewport.from_bottom_px + size[1],
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
        }
        Ok(())
    }
    unsafe fn free_target(&mut self, gl: &glow::Context) {
        if let Some((fbo, texture, _)) = self.target.take() {
            unsafe {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(texture);
            }
        }
    }
    pub fn shutdown(&mut self, gl: &glow::Context) {
        unsafe {
            self.free_target(gl);
            if !self.render.is_null() {
                (self.api.render_free)(self.render);
                self.render = std::ptr::null_mut();
            }
        }
    }
}
impl Drop for Video {
    fn drop(&mut self) {
        // Normal shutdown frees the GL renderer in on_exit before destroying mpv.
        if self.render.is_null() {
            unsafe {
                (self.api.destroy)(self.handle);
            }
        }
    }
}

pub fn canvas(ui: &mut egui::Ui, video: Arc<Mutex<Video>>) {
    let (rect, _) = ui.allocate_exact_size(
        ui.available_size().max(egui::vec2(1.0, 1.0)),
        egui::Sense::hover(),
    );
    ui.painter().add(egui::PaintCallback {
        rect,
        callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
            let mut video = video.lock().unwrap();
            if let Err(error) = unsafe { video.paint(info, painter) } {
                video.state.error = Some(error);
            }
            // Restore the framebuffer even if mpv or target allocation failed.
            unsafe {
                painter
                    .gl()
                    .bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
            }
        })),
    });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
}
