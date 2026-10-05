#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod blob;
mod capture;
mod config;
mod db;
mod doc;
mod log;
mod main_view;
mod text;
mod markup;
mod theme;
mod tray;
mod win;

fn main() {
    // One instance only; a second launch just exits.
    let Some(_instance) = win::single_instance() else {
        return;
    };

    let dir = config::data_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot create {}: {e}", dir.display());
        return;
    }
    log::init(&dir);
    let cfg = config::load(&dir);

    // Pre-rename installs keep their database file as-is (renaming a live WAL database is not worth the risk).
    let legacy = dir.join("omniware.db");
    let db_path = if legacy.exists() { legacy } else { dir.join("omniaware.db") };
    let db = match db::Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            log::error(format!("database: {e}"));
            return;
        }
    };

    let renderer = match cfg.renderer.to_ascii_lowercase().as_str() {
        "glow" | "opengl" => eframe::Renderer::Glow,
        _ => eframe::Renderer::Wgpu,
    };

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(app::TITLE)
            .with_inner_size([cfg.window.width, cfg.window.height])
            .with_min_inner_size([360.0, 200.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_visible(false),
        centered: true,
        renderer,
        ..Default::default()
    };

    let result = eframe::run_native(
        app::TITLE,
        opts,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, cfg, db, dir)))),
    );
    if let Err(e) = result {
        log::error(format!("eframe: {e}"));
    }
}
