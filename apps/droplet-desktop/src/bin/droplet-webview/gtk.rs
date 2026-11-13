//! GTK 3 + WebKitGTK 4.1 bindings loaded with `dlopen`, and the helper's main loop.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_uint, c_ulong, c_void};

use crate::{one_line, take_lines, Command, Options};

type Ptr = *mut c_void;
type GCallback = unsafe extern "C" fn();

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;
const G_IO_IN: c_uint = 1;
const G_IO_HUP: c_uint = 16;
const G_IO_ERR: c_uint = 8;
const GTK_WINDOW_TOPLEVEL: c_int = 0;
const WEBKIT_COOKIE_PERSISTENT_STORAGE_SQLITE: c_int = 1;
const WEBKIT_CACHE_MODEL_WEB_BROWSER: c_int = 1;

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> Ptr;
    fn dlsym(handle: Ptr, symbol: *const c_char) -> Ptr;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
}

macro_rules! api {
    ($( $lib:ident { $( $name:ident : $ty:ty ; )* } )*) => {
        struct Api { $($( $name: $ty, )*)* }

        impl Api {
            fn load() -> Result<Self, String> {
                $( let $lib = open(stringify!($lib))?; )*
                // SAFETY: each symbol is declared with its C prototype from the GTK/WebKit headers.
                unsafe {
                    Ok(Self { $($( $name: std::mem::transmute::<Ptr, $ty>(sym($lib, stringify!($name))?), )*)* })
                }
            }
        }
    };
}

api! {
    glib {
        g_unix_fd_add: unsafe extern "C" fn(c_int, c_uint, unsafe extern "C" fn(c_int, c_uint, Ptr) -> c_int, Ptr) -> c_uint;
    }
    gobject {
        g_signal_connect_data: unsafe extern "C" fn(Ptr, *const c_char, GCallback, Ptr, Ptr, c_int) -> c_ulong;
    }
    gtk {
        gtk_init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char);
        gtk_main: unsafe extern "C" fn();
        gtk_main_quit: unsafe extern "C" fn();
        gtk_window_new: unsafe extern "C" fn(c_int) -> Ptr;
        gtk_plug_new: unsafe extern "C" fn(c_ulong) -> Ptr;
        gtk_plug_get_id: unsafe extern "C" fn(Ptr) -> c_ulong;
        gtk_window_set_default_size: unsafe extern "C" fn(Ptr, c_int, c_int);
        gtk_window_resize: unsafe extern "C" fn(Ptr, c_int, c_int);
        gtk_window_set_title: unsafe extern "C" fn(Ptr, *const c_char);
        gtk_container_add: unsafe extern "C" fn(Ptr, Ptr);
        gtk_widget_show_all: unsafe extern "C" fn(Ptr);
        gtk_widget_grab_focus: unsafe extern "C" fn(Ptr);
    }
    webkit {
        webkit_website_data_manager_new: unsafe extern "C" fn(*const c_char, ...) -> Ptr;
        webkit_website_data_manager_get_cookie_manager: unsafe extern "C" fn(Ptr) -> Ptr;
        webkit_cookie_manager_set_persistent_storage: unsafe extern "C" fn(Ptr, *const c_char, c_int);
        webkit_web_context_new_with_website_data_manager: unsafe extern "C" fn(Ptr) -> Ptr;
        webkit_web_context_set_cache_model: unsafe extern "C" fn(Ptr, c_int);
        webkit_web_view_new_with_context: unsafe extern "C" fn(Ptr) -> Ptr;
        webkit_web_view_load_uri: unsafe extern "C" fn(Ptr, *const c_char);
        webkit_web_view_get_uri: unsafe extern "C" fn(Ptr) -> *const c_char;
        webkit_web_view_get_title: unsafe extern "C" fn(Ptr) -> *const c_char;
        webkit_web_view_get_estimated_load_progress: unsafe extern "C" fn(Ptr) -> c_double;
        webkit_web_view_go_back: unsafe extern "C" fn(Ptr);
        webkit_web_view_go_forward: unsafe extern "C" fn(Ptr);
        webkit_web_view_reload: unsafe extern "C" fn(Ptr);
        webkit_web_view_stop_loading: unsafe extern "C" fn(Ptr);
    }
}

fn open(lib: &str) -> Result<Ptr, String> {
    let names: &[&CStr] = match lib {
        "glib" => &[c"libglib-2.0.so.0"],
        "gobject" => &[c"libgobject-2.0.so.0"],
        "gtk" => &[c"libgtk-3.so.0"],
        _ => &[c"libwebkit2gtk-4.1.so.0", c"libwebkit2gtk-4.0.so.37"],
    };
    for name in names {
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
        if !handle.is_null() {
            return Ok(handle);
        }
    }
    Err(format!("{} not found", names[0].to_string_lossy()))
}

fn sym(handle: Ptr, name: &str) -> Result<Ptr, String> {
    let cname = CString::new(name).map_err(|_| "bad symbol name".to_string())?;
    let ptr = unsafe { dlsym(handle, cname.as_ptr()) };
    if ptr.is_null() {
        Err(format!("missing symbol {name}"))
    } else {
        Ok(ptr)
    }
}

struct App {
    api: Api,
    window: Ptr,
    view: Ptr,
    stdin_buf: Vec<u8>,
}

fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

unsafe fn text(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        one_line(&CStr::from_ptr(ptr).to_string_lossy())
    }
}

unsafe fn connect(app: &App, instance: Ptr, signal: &CStr, handler: GCallback) {
    (app.api.g_signal_connect_data)(
        instance,
        signal.as_ptr(),
        handler,
        app as *const App as Ptr,
        std::ptr::null_mut(),
        0,
    );
}

unsafe extern "C" fn on_title(_view: Ptr, _pspec: Ptr, data: Ptr) {
    let app = &*(data as *const App);
    let title = text((app.api.webkit_web_view_get_title)(app.view));
    println!("title {title}");
    (app.api.gtk_window_set_title)(app.window, cstring(&title).as_ptr());
}

unsafe extern "C" fn on_uri(_view: Ptr, _pspec: Ptr, data: Ptr) {
    let app = &*(data as *const App);
    println!("uri {}", text((app.api.webkit_web_view_get_uri)(app.view)));
}

unsafe extern "C" fn on_progress(_view: Ptr, _pspec: Ptr, data: Ptr) {
    let app = &*(data as *const App);
    let progress = (app.api.webkit_web_view_get_estimated_load_progress)(app.view);
    println!("progress {progress:.2}");
}

unsafe extern "C" fn on_load_changed(_view: Ptr, event: c_int, _data: Ptr) {
    // WebKitLoadEvent: STARTED=0, REDIRECTED=1, COMMITTED=2, FINISHED=3.
    match event {
        0 => println!("load started"),
        2 => println!("load committed"),
        3 => println!("load finished"),
        _ => {}
    }
}

unsafe extern "C" fn on_destroy(_widget: Ptr, data: Ptr) {
    let app = &*(data as *const App);
    (app.api.gtk_main_quit)();
}

unsafe extern "C" fn on_stdin(fd: c_int, condition: c_uint, data: Ptr) -> c_int {
    let app = &mut *(data as *mut App);
    let mut chunk = [0u8; 4096];
    let n = read(fd, chunk.as_mut_ptr().cast(), chunk.len());
    let _ = condition;
    if n <= 0 {
        // The browser closed our stdin: it is gone or wants us gone.
        (app.api.gtk_main_quit)();
        return 0;
    }
    app.stdin_buf.extend_from_slice(&chunk[..n as usize]);
    for line in take_lines(&mut app.stdin_buf) {
        match Command::parse(&line) {
            Some(Command::Load(url)) => {
                (app.api.webkit_web_view_load_uri)(app.view, cstring(&url).as_ptr())
            }
            Some(Command::Back) => (app.api.webkit_web_view_go_back)(app.view),
            Some(Command::Forward) => (app.api.webkit_web_view_go_forward)(app.view),
            Some(Command::Reload) => (app.api.webkit_web_view_reload)(app.view),
            Some(Command::Stop) => (app.api.webkit_web_view_stop_loading)(app.view),
            Some(Command::Resize(w, h)) => {
                (app.api.gtk_window_resize)(app.window, w.max(1), h.max(1))
            }
            Some(Command::Quit) => {
                (app.api.gtk_main_quit)();
                return 0;
            }
            None => eprintln!("droplet-webview: ignoring command {line:?}"),
        }
    }
    1
}

fn default_data_dir() -> String {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return format!("{}/droplet/webkit", dir.to_string_lossy());
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.local/share/droplet/webkit")
}

fn cache_dir() -> String {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        return format!("{}/droplet/webkit", dir.to_string_lossy());
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.cache/droplet/webkit")
}

pub fn run(opts: Options) -> Result<(), String> {
    if opts.embed.is_some() {
        // XEmbed only exists on X11; under a Wayland session this runs through XWayland.
        std::env::set_var("GDK_BACKEND", "x11");
    }
    let api = Api::load()?;
    let data_dir = opts.data_dir.clone().unwrap_or_else(default_data_dir);
    let cache = cache_dir();
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::create_dir_all(&cache);

    unsafe {
        (api.gtk_init)(std::ptr::null_mut(), std::ptr::null_mut());

        let data_c = cstring(&data_dir);
        let cache_c = cstring(&cache);
        let manager = (api.webkit_website_data_manager_new)(
            c"base-data-directory".as_ptr(),
            data_c.as_ptr(),
            c"base-cache-directory".as_ptr(),
            cache_c.as_ptr(),
            std::ptr::null::<c_char>(),
        );
        // Logins must survive a restart: keep cookies in SQLite next to the other data.
        let cookies = cstring(&format!("{data_dir}/cookies.sqlite"));
        (api.webkit_cookie_manager_set_persistent_storage)(
            (api.webkit_website_data_manager_get_cookie_manager)(manager),
            cookies.as_ptr(),
            WEBKIT_COOKIE_PERSISTENT_STORAGE_SQLITE,
        );
        let context = (api.webkit_web_context_new_with_website_data_manager)(manager);
        (api.webkit_web_context_set_cache_model)(context, WEBKIT_CACHE_MODEL_WEB_BROWSER);
        let view = (api.webkit_web_view_new_with_context)(context);

        let window = match opts.embed {
            Some(xid) => (api.gtk_plug_new)(xid as c_ulong),
            None => {
                let w = (api.gtk_window_new)(GTK_WINDOW_TOPLEVEL);
                (api.gtk_window_set_default_size)(w, 1280, 800);
                w
            }
        };

        let app: &'static mut App = Box::leak(Box::new(App {
            api,
            window,
            view,
            stdin_buf: Vec::new(),
        }));
        // GObject calls every handler through the generic `GCallback` type.
        type Notify = unsafe extern "C" fn(Ptr, Ptr, Ptr);
        type LoadChanged = unsafe extern "C" fn(Ptr, c_int, Ptr);
        type Destroy = unsafe extern "C" fn(Ptr, Ptr);
        let notify = |f: Notify| std::mem::transmute::<Notify, GCallback>(f);
        let on_load = std::mem::transmute::<LoadChanged, GCallback>(on_load_changed);
        let on_destroy = std::mem::transmute::<Destroy, GCallback>(on_destroy);
        connect(app, view, c"notify::title", notify(on_title));
        connect(app, view, c"notify::uri", notify(on_uri));
        connect(
            app,
            view,
            c"notify::estimated-load-progress",
            notify(on_progress),
        );
        connect(app, view, c"load-changed", on_load);
        connect(app, window, c"destroy", on_destroy);
        (app.api.g_unix_fd_add)(
            0,
            G_IO_IN | G_IO_HUP | G_IO_ERR,
            on_stdin,
            app as *mut App as Ptr,
        );

        (app.api.gtk_container_add)(window, view);
        (app.api.gtk_widget_show_all)(window);
        (app.api.gtk_widget_grab_focus)(view);
        if opts.embed.is_some() {
            println!("ready {:#x}", (app.api.gtk_plug_get_id)(window));
        }
        (app.api.webkit_web_view_load_uri)(view, cstring(&opts.url).as_ptr());
        (app.api.gtk_main)();
    }
    Ok(())
}
