#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

mod catalog;
mod window_state;
mod indexer;
use std::sync::Arc;
use tauri::Manager;
// Geometry is restored while the window is hidden.
// The frontend calls this only after its initial content and theme have rendered.
#[tauri::command]
async fn app_ready(window: tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}
#[tauri::command]
async fn api(
    state: tauri::State<'_, Arc<catalog::App>>,
    op: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || app.dispatch(&op, &args))
        .await
        .map_err(|e| e.to_string())?
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args
        .get(1)
        .is_some_and(|a| a == "--collect-index" || a == "--build-index")
        && args.len() != 4
    {
        eprintln!(
            "Usage: danbooru-tag-indexer --build-index RAW OUTPUT | --collect-index DATA_DIR SEED"
        );
        std::process::exit(2);
    }
    if args.get(1).map(String::as_str) == Some("--collect-index") {
        let app = Arc::new(
            catalog::App::new(
                std::path::PathBuf::from(&args[2]),
                std::path::PathBuf::from(&args[3]),
            )
            .expect("development catalog"),
        );
        indexer::control(app.clone(), "start").expect("start development collection");
        while app.running.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        let job = app.job.lock().unwrap();
        println!("{}", *job);
        if job["status"] != "done" {
            std::process::exit(1)
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--build-index") {
        let raw = rusqlite::Connection::open(&args[2]).expect("raw database");
        let path = std::path::Path::new(&args[3]);
        indexer::build(
            &raw,
            path,
            || Ok(()),
            |p, n| {
                if p % 5000 == 0 {
                    println!("{p} pages, {n} nodes");
                }
            },
        )
        .expect("build index");
        catalog::validate(path).expect("validate index");
        println!("Index validated");
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            window_state::setup(app)?;
            let dir = app.path().app_data_dir()?;
            let seed = app.path().resource_dir()?.join("resources/codex-v1.sqlite");
            app.manage(Arc::new(
                catalog::App::new(dir, seed).map_err(std::io::Error::other)?,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![api, app_ready])
        .run(tauri::generate_context!())
        .expect("Could not run Danbooru Tag Indexer");
}

