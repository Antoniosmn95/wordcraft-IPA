//! WordCraft for iPadOS.
//!
//! The Rust engine, command registry and egui interface are the same ones the desktop and web
//! builds use. This crate is a static library loaded by the Swift/UIKit app in `ios/`; the Swift
//! entry point calls [`wc_ios_main`], and winit drives the iOS application lifecycle.
//!
//! Only the iOS target builds this crate. Other workspace targets see an empty library so the
//! normal Linux/macOS/Windows/FreeBSD gates keep working without an iOS SDK.
#![cfg(target_os = "ios")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::ffi::c_int;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use log::{LevelFilter, Log, Metadata, Record};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Inbox, Services, UiState, WordApp};

/// Asks Swift to show the system document picker. `purpose` is 0 for a document, 1 for a
/// picture, 2 for a font, 3 for recipient data and 4 for Word sources; the picked file is copied
/// into `Documents/Inbox`, where [`IosApp`] imports it.
pub type OpenPicker = extern "C" fn(u32);

static OPEN_PICKER: OnceLock<OpenPicker> = OnceLock::new();

/// The entry point the Swift `main.swift` calls.
///
/// On iOS winit calls `UIApplicationMain`, which does not return while the app is in the
/// foreground, so this follows the winit iOS contract: build the event loop and hand control to
/// UIKit.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn wc_ios_main(open_picker: Option<OpenPicker>) -> c_int {
    install_logger();
    if let Some(picker) = open_picker {
        let _ = OPEN_PICKER.set(picker);
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => {
            log::error!("WordCraft failed to start: {e}");
            1
        }
        Err(_) => {
            log::error!("WordCraft stopped after an unexpected internal error");
            1
        }
    }
}

fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WordCraft")
            .with_app_id("ai.storyteller.wordcraft")
            .with_inner_size([1180.0, 820.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("WordCraft", options, Box::new(|cc| Ok(Box::new(IosApp::new(cc)))))
}

struct IosApp {
    app: WordApp,
    inbox: Inbox,
    last_inbox_scan: Instant,
}

impl IosApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let region = wordcraft_ui_egui::i18n::system_region();
        let doc = wordcraft_engine::blank_document(&region);
        let mut session = Session::new(doc);
        session.system_region = region;
        let inbox = Inbox::default();
        let services = services(inbox.clone());
        let mut app = WordApp::new(session, services);
        app.set_window_builder(egui::ViewportBuilder::default().with_inner_size([1180.0, 820.0]));
        app.integrated_titlebar = false;
        load_user_fonts();
        load_prefs(&mut app);
        load_user_dictionary(&mut app);
        app.watch_desktop_theme(&cc.egui_ctx);
        if let Some(dir) = settings_dir() {
            let ctx = cc.egui_ctx.clone();
            if let Err(e) = app.session.enable_recovery(dir.join("recovery"), Arc::new(move || ctx.request_repaint())) {
                log::warn!("local recovery unavailable: {e}");
            }
        }
        let mut app = Self { app, inbox, last_inbox_scan: Instant::now() - Duration::from_secs(60) };
        app.scan_inbox();
        if app.app.session.path.is_none() {
            app.app.start_screen();
        }
        app
    }

    /// iOS copies a document opened from another app into `Documents/Inbox`. Move each file into
    /// `Documents` so it has a stable path to save back to, and hand its bytes to the app.
    fn scan_inbox(&mut self) {
        let Some(documents) = documents_dir() else { return };
        let inbox_dir = documents.join("Inbox");
        let Ok(entries) = std::fs::read_dir(&inbox_dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let destination = unique_path(&documents, name);
            if std::fs::rename(&path, &destination).is_err() {
                let Ok(bytes) = std::fs::read(&path) else { continue };
                if std::fs::write(&destination, &bytes).is_err() {
                    continue;
                }
                let _ = std::fs::remove_file(&path);
            }
            let Ok(bytes) = std::fs::read(&destination) else { continue };
            let shown = destination.to_string_lossy().into_owned();
            if let Ok(mut queue) = self.inbox.lock() {
                queue.push((shown, bytes));
            }
        }
    }
}

impl eframe::App for IosApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_inbox_scan.elapsed() >= Duration::from_millis(500) {
            self.last_inbox_scan = Instant::now();
            self.scan_inbox();
        }
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }

    fn on_exit(&mut self) {
        self.app.session.finish_recovery();
        let _ = save_prefs(&self.app);
    }
}

fn services(inbox: Inbox) -> Services {
    Services {
        inbox: Some(inbox),
        pick_save: Some(Box::new(save_path)),
        open_async: OPEN_PICKER.get().map(|picker| {
            let picker = *picker;
            Box::new(move |purpose: &str| picker(purpose_code(purpose))) as Box<dyn Fn(&str)>
        }),
        // iPadOS shows its own document picker through the Swift bridge. Until a pick arrives,
        // the built-in browser lets the person reach the app's Documents folder.
        templates_dir: settings_dir().map(|d| d.join("Templates")),
        save_prefs: Some(Box::new(save_prefs)),
        ..Default::default()
    }
}

/// The Swift picker's filter for a file purpose.
fn purpose_code(purpose: &str) -> u32 {
    match purpose {
        "picture" => 1,
        "font" => 2,
        "recipients" => 3,
        "sources" => 4,
        _ => 0,
    }
}

/// A path in the app's Documents folder for Save As and exports. An existing name gets ` 2`,
/// ` 3`… so a save never silently replaces a different document.
fn save_path(name: &str) -> Option<String> {
    let documents = documents_dir()?;
    std::fs::create_dir_all(&documents).ok()?;
    let name = Path::new(name).file_name()?.to_string_lossy().into_owned();
    Some(unique_path(&documents, &name).to_string_lossy().into_owned())
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Document".into());
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 2..10_000 {
        let file = match &ext {
            Some(ext) => format!("{stem} {n}.{ext}"),
            None => format!("{stem} {n}"),
        };
        let candidate = dir.join(file);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents"))
}

fn settings_dir() -> Option<PathBuf> {
    wordcraft_control_key::settings_dir()
}

fn prefs_path() -> Option<PathBuf> {
    settings_dir().map(|d| d.join("ui.json"))
}

fn user_dict_path() -> Option<PathBuf> {
    settings_dir().map(|d| d.join(Session::USER_DICT_FILE))
}

fn load_prefs(app: &mut WordApp) {
    if let Some(path) = prefs_path()
        && let Ok(bytes) = std::fs::read(&path)
        && let Some(ui) = UiState::from_json(&bytes)
    {
        app.apply_prefs(ui);
    }
}

fn save_prefs(app: &WordApp) -> Result<bool, String> {
    let Some(path) = prefs_path() else { return Ok(false) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(&app.prefs()).map_err(|e| format!("cannot serialize preferences: {e}"))?;
    write_atomic(&path, &bytes).map_err(|e| format!("cannot save {}: {e}", path.display()))?;
    Ok(true)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "ui.json".into());
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path))?;
    Ok(())
}

fn load_user_fonts() {
    let Some(dir) = settings_dir().map(|d| d.join("fonts")) else { return };
    let (loaded, skipped) = wordcraft_fonts::FontDb::global().load_user_fonts(&dir);
    if !loaded.is_empty() || !skipped.is_empty() {
        log::info!("user fonts: {} loaded, {} skipped from {}", loaded.len(), skipped.len(), dir.display());
    }
}

fn load_user_dictionary(app: &mut WordApp) {
    if let Some(path) = user_dict_path() {
        wordcraft_proof::load_user_dictionary(&path);
        app.session.user_dict_path = Some(path);
    }
}

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let _ = writeln!(std::io::stderr(), "[{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

fn install_logger() {
    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(LevelFilter::Info);
}
