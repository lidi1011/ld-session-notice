mod claude_usage;
mod app_state;
mod channels;
mod codex_usage;
mod commands;
mod domain;
mod hooks;
mod mochi_voice;
mod pet;
mod rules;
mod secret_store;
mod server;
mod storage;

use app_state::AppState;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let state = tauri::async_runtime::block_on(AppState::initialize())?;
            let server_state = state.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = server::run(server_state).await {
                    eprintln!("Notice local server stopped: {error}");
                }
            });
            let widget_state = state.clone();
            let widget_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match storage::bool_setting(&widget_state.pool, "traffic_widget_enabled", true)
                    .await
                {
                    Ok(true) => {
                        if let Err(error) =
                            commands::show_traffic_widget_for_state(&widget_app, &widget_state)
                                .await
                        {
                            eprintln!("Notice traffic widget failed to open: {error}");
                        }
                    }
                    Ok(false) => {}
                    Err(error) => eprintln!("Notice traffic widget setting failed: {error}"),
                }
            });
            mochi_voice::start(state.clone());
            let show = MenuItemBuilder::with_id("show", "Show Notice").build(app)?;
            let show_widget =
                MenuItemBuilder::with_id("show_widget", "Show Traffic Widget").build(app)?;
            let hide_widget =
                MenuItemBuilder::with_id("hide_widget", "Hide Traffic Widget").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show, &show_widget, &hide_widget, &quit])
                .build()?;
            let mut tray = TrayIconBuilder::new().menu(&menu);
            #[cfg(target_os = "macos")]
            {
                // Template rendering keeps the status-light glyph legible in both appearances.
                tray = tray
                    .icon(tauri::include_image!("icons/tray-status-light.png"))
                    .icon_as_template(true);
            }
            #[cfg(not(target_os = "macos"))]
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id().as_ref() {
                "show" => show_main_window(app),
                "show_widget" => {
                    if let Err(error) = commands::show_traffic_widget(app, true) {
                        eprintln!("Notice traffic widget failed to open: {error}");
                    }
                }
                "hide_widget" => {
                    if let Some(window) = app.get_webview_window("traffic-widget") {
                        let _ = window.hide();
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main_window(tray.app_handle());
                }
            })
            .build(app)?;
            app.manage(state);
            claude_usage::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_dashboard_summary,
            commands::list_events,
            commands::clear_events,
            commands::refresh_runtime_status,
            commands::get_app_locale,
            commands::set_app_locale,
            commands::get_autostart_enabled,
            commands::set_autostart_enabled,
            commands::get_channel_config,
            commands::save_feishu_config,
            commands::test_feishu_channel,
            commands::set_feishu_enabled,
            commands::get_claude_hook_status,
            commands::preview_claude_hook_install,
            commands::install_claude_hooks,
            commands::uninstall_claude_hooks,
            commands::get_claude_usage_status,
            commands::open_claude_account,
            commands::refresh_claude_usage,
            commands::get_hook_status,
            commands::preview_hook_install,
            commands::install_codex_hooks,
            commands::uninstall_codex_hooks,
            commands::list_pending_approvals,
            commands::resolve_approval,
            commands::get_traffic_widget_status,
            commands::set_traffic_widget_enabled,
            commands::set_traffic_widget_always_on_top,
            commands::set_traffic_widget_manual_override,
            commands::get_pet_config,
            commands::save_pet_config,
            commands::test_pet_connection,
            commands::get_mochi_voice_config,
            commands::save_mochi_voice_config
        ])
        .build(tauri::generate_context!())
        .expect("error while building Notice");

    app.run(|app, event| {
        if let RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } = &event
        {
            if label == "main" {
                api.prevent_close();
                if let Some(window) = app.get_webview_window(&label) {
                    let _ = window.hide();
                }
            }
        }

        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            show_main_window(app);
        }
    });
}

fn show_main_window(app: &tauri::AppHandle) {
    let window = app.get_webview_window("main").or_else(|| {
        WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("Notice")
            .inner_size(1120.0, 760.0)
            .min_inner_size(980.0, 720.0)
            .build()
            .ok()
    });

    if let Some(window) = window {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
