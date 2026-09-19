use crate::state::State;
use anyhow::Result;
use libloading::{Library, Symbol};
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type PluginInitFn = unsafe extern "C" fn() -> i32;
type PluginUpdateFn = unsafe extern "C" fn() -> *mut std::os::raw::c_char;
type PluginFreeStringFn = unsafe extern "C" fn(*mut std::os::raw::c_char);
type PluginShutdownFn = unsafe extern "C" fn() -> i32;

pub struct LoadedPlugin {
    name: String,
    library: Library,
    update_fn: PluginUpdateFn,
    free_string_fn: PluginFreeStringFn,
    shutdown_fn: PluginShutdownFn,
}

impl LoadedPlugin {
    fn new(library: Library) -> Result<Self> {
        // Get all function pointers first (borrowing library)
        let name_ptr = {
            let name_fn: Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
                unsafe { library.get(b"plugin_name")? };
            unsafe { name_fn() }
        };
        let version_ptr = {
            let version_fn: Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
                unsafe { library.get(b"plugin_version")? };
            unsafe { version_fn() }
        };
        let desc_ptr = {
            let desc_fn: Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
                unsafe { library.get(b"plugin_description")? };
            unsafe { desc_fn() }
        };

        let init_result = {
            let init_fn: Symbol<PluginInitFn> = unsafe { library.get(b"plugin_init")? };
            unsafe { init_fn() }
        };
        if init_result != 0 {
            return Err(anyhow::anyhow!(
                "Plugin init failed with code {}",
                init_result
            ));
        }

        // Now get the function pointers we'll store - extract raw pointers first
        let update_fn: PluginUpdateFn = {
            let sym: Symbol<PluginUpdateFn> = unsafe { library.get(b"plugin_update")? };
            unsafe { *sym }
        };
        let free_string_fn: PluginFreeStringFn = {
            let sym: Symbol<PluginFreeStringFn> = unsafe { library.get(b"plugin_free_string")? };
            unsafe { *sym }
        };
        let shutdown_fn: PluginShutdownFn = {
            let sym: Symbol<PluginShutdownFn> = unsafe { library.get(b"plugin_shutdown")? };
            unsafe { *sym }
        };

        let name = unsafe { CStr::from_ptr(name_ptr).to_string_lossy().into_owned() };
        let version = unsafe { CStr::from_ptr(version_ptr).to_string_lossy().into_owned() };
        let description = unsafe { CStr::from_ptr(desc_ptr).to_string_lossy().into_owned() };

        println!("Loading plugin: {} v{} - {}", name, version, description);

        Ok(Self {
            name,
            library,
            update_fn,
            free_string_fn,
            shutdown_fn,
        })
    }

    fn update(&self, state: &mut State) -> Result<()> {
        let json_ptr = unsafe { (self.update_fn)() };
        if json_ptr.is_null() {
            return Ok(());
        }

        let json_str = unsafe { CStr::from_ptr(json_ptr).to_string_lossy().into_owned() };
        unsafe { (self.free_string_fn)(json_ptr) };

        let value: serde_json::Value = serde_json::from_str(&json_str)?;
        state.plugin_data.insert(self.name.clone(), value);

        Ok(())
    }

    fn shutdown(&self) -> Result<()> {
        let result = unsafe { (self.shutdown_fn)() };
        if result != 0 {
            eprintln!("Plugin '{}' shutdown returned code {}", self.name, result);
        }
        Ok(())
    }
}

pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
    plugin_dir: PathBuf,
}

impl PluginManager {
    pub fn new(plugin_dir: impl AsRef<Path>) -> Self {
        Self {
            plugins: Vec::new(),
            plugin_dir: plugin_dir.as_ref().to_path_buf(),
        }
    }

    pub fn load_all(&mut self) -> Result<()> {
        if !self.plugin_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.plugin_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("so") {
                self.load_plugin(&path)?;
            }
        }
        Ok(())
    }

    fn load_plugin(&mut self, path: &Path) -> Result<()> {
        let library = unsafe { Library::new(path)? };
        let plugin = LoadedPlugin::new(library)?;
        self.plugins.push(plugin);
        Ok(())
    }

    pub fn update_all(&mut self, state: &mut State) -> Result<()> {
        for plugin in &mut self.plugins {
            if let Err(e) = plugin.update(state) {
                eprintln!("Plugin '{}' update error: {}", plugin.name, e);
            }
        }
        Ok(())
    }

    pub fn shutdown_all(&mut self) -> Result<()> {
        for plugin in self.plugins.drain(..) {
            if let Err(e) = plugin.shutdown() {
                eprintln!("Plugin '{}' shutdown error: {}", plugin.name, e);
            }
        }
        Ok(())
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        let _ = self.shutdown_all();
    }
}

lazy_static::lazy_static! {
    static ref GLOBAL_PLUGIN_MANAGER: Arc<Mutex<Option<PluginManager>>> = Arc::new(Mutex::new(None));
}

pub fn init_global_plugin_manager(plugin_dir: impl AsRef<Path>) -> Result<()> {
    let mut guard = GLOBAL_PLUGIN_MANAGER.lock().unwrap();
    *guard = Some(PluginManager::new(plugin_dir));
    guard.as_mut().unwrap().load_all()
}

pub fn update_global_plugins(state: &mut State) -> Result<()> {
    if let Some(manager) = GLOBAL_PLUGIN_MANAGER.lock().unwrap().as_mut() {
        manager.update_all(state)
    } else {
        Ok(())
    }
}

pub fn shutdown_global_plugins() -> Result<()> {
    if let Some(mut manager) = GLOBAL_PLUGIN_MANAGER.lock().unwrap().take() {
        manager.shutdown_all()
    } else {
        Ok(())
    }
}
