#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod commands;
mod db;
mod models;
mod scanner;

use db::Db;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let database = db::init_db(app.handle())?;
            app.manage(database);
            app.manage(scanner::ScanState::new());
            Ok(())
        })
        // Protocolo custom `thumb://{id}` para servir miniaturas como BLOB
        // sin serializarlas por IPC (crítico con catálogos enormes).
        .register_uri_scheme_protocol("thumb", |ctx, request| {
            let id: i64 = request
                .uri()
                .path()
                .trim_start_matches('/')
                .parse()
                .unwrap_or(-1);
            let data = if id >= 0 {
                let state = ctx.app_handle().state::<Db>();
                let conn = match state.0.lock() {
                    Ok(c) => c,
                    Err(_) => return no_thumb(),
                };
                commands::thumb_blob(&conn, id).unwrap_or_default()
            } else {
                Vec::new()
            };
            tauri::http::Response::builder()
                .header("Content-Type", "image/jpeg")
                .header("Cache-Control", "max-age=31536000, immutable")
                .body(data)
                .unwrap()
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_catalogs,
            commands::create_catalog,
            commands::delete_catalog,
            commands::list_groups,
            commands::create_group,
            commands::update_group,
            commands::delete_group,
            commands::set_sidebar_layout,
            commands::cancel_scan,
            commands::is_scanning,
            commands::get_children,
            commands::get_ancestors,
            commands::search,
            commands::get_stats,
            commands::export_backup,
            commands::import_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error mientras se ejecutaba ZorCatalog");
}

fn no_thumb() -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(404)
        .body(Vec::new())
        .unwrap()
}
