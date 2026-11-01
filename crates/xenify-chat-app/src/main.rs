//! Xenify Chat: the desktop app. The screens are HTML in `ui/`; this side talks
//! to the chat server and runs the spelling and grammar checker.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod calendar;
mod calls;
mod check;
mod files;
mod host;
mod mail;
mod net;
mod notify;
mod roots;
mod servermail;
mod tray;
#[cfg(feature = "voice")]
mod voice;
#[cfg(not(feature = "voice"))]
#[path = "novoice.rs"]
mod voice;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    tauri::Builder::default()
        // Opened again (from the Hub, the menu, a shortcut): the running
        // Chat comes forward and the new one ends.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // (Started with the computer, in the tray: nothing to bring forward.)
            if !args.iter().any(|a| a == tray::HIDDEN_ARG) {
                tray::show(app);
            }
        }))
        .manage(net::Net::new(&args))
        .manage(check::Check::new())
        .manage(mail::Mail::default())
        .manage(servermail::ServerMail::default())
        .manage(files::Files::default())
        .manage(voice::Voice::default())
        .manage(calls::Grab::default())
        // Calls: the camera, microphone and screen, for this app's own window.
        .on_permission_request(|_, kind| {
            use tauri::webview::{PermissionKind, PermissionResponse};
            match kind {
                PermissionKind::Camera | PermissionKind::Microphone | PermissionKind::DisplayCapture => {
                    PermissionResponse::Allow
                }
                _ => PermissionResponse::Default,
            }
        })
        .setup(|app| {
            calls::enable_media(app);
            // In the tray, so closing the window keeps Chat running.
            if let Some(t) = tray::start(app.handle()) {
                tauri::Manager::manage(app, t);
            }
            if std::env::args().any(|a| a == tray::HIDDEN_ARG)
                && tauri::Manager::try_state::<tray::Tray>(app).is_some()
                && let Some(w) = tauri::Manager::get_webview_window(app, "main")
            {
                let _ = w.hide();
            }
            // Meetings, your calendar and address books: the core shared with
            // the browser build.
            let chat = host::chat(app.handle());
            tauri::Manager::manage(app, chat);
            Ok(())
        })
        // Back at the window: the Hub may have signed in elsewhere, or out.
        .on_window_event(|w, e| {
            // Closing keeps it in the tray (when there is one).
            if let tauri::WindowEvent::CloseRequested { api, .. } = e
                && tauri::Manager::try_state::<tray::Tray>(tauri::Manager::app_handle(w)).is_some()
            {
                api.prevent_close();
                let _ = w.hide();
            }
            if let tauri::WindowEvent::Focused(true) = e {
                use tauri::Manager;
                let app = w.app_handle();
                if app.state::<net::Net>().follow_hub()
                    && let Some(v) = app.get_webview_window(w.label())
                {
                    let _ = v.eval("location.reload()");
                }
            }
        })
        .register_asynchronous_uri_scheme_protocol("xfile", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            std::thread::spawn(move || responder.respond(files::protocol(&app, &request)));
        })
        .invoke_handler(handler(tauri::generate_handler![
            open_hub,
            calls::screen_x11,
            calls::screen_grab_start,
            calls::screen_grab_frame,
            calls::screen_grab_stop,
            net::start,
            net::sign_in,
            net::second_code,
            net::second_email,
            net::mfa_setup_app,
            net::mfa_setup_email,
            net::mfa_setup_finish,
            net::mfa_state,
            net::mfa_off,
            net::mfa_backup,
            net::link_info,
            net::send,
            net::server_info,
            net::holidays,
            net::secret_new,
            net::sign_in_start,
            net::forgot,
            net::sign_out,
            net::change_server,
            net::certificate,
            net::use_hub,
            net::log,
            files::file_send_path,
            files::file_begin,
            files::file_part,
            files::file_end,
            files::file_save,
            files::board_new,
            files::board_open_in_draw,
            files::board_session,
            files::save_download,
            check::check,
            check::words,
            check::add_word,
            check::remove_word,
            check::spotter_state,
            check::checker_credits,
            notify::notify,
            notify::load_settings,
            notify::save_settings,
            tray::attention,
            tray::autostart_get,
            tray::autostart_set,
            tray::close_window,
            tray::quit,
            mail::mail_accounts,
            mail::mail_look_up,
            mail::mail_add,
            mail::mail_remove,
            mail::mail_folders,
            mail::mail_list,
            mail::mail_search,
            mail::mail_open,
            mail::mail_flag,
            mail::mail_delete,
            mail::mail_move,
            mail::mail_sync_now,
            mail::mail_send,
            mail::mail_save_attachment,
            mail::open_link,
            mail::mail_drafts,
            mail::mail_draft_save,
            mail::mail_draft_delete,
            mail::mail_answer_invite,
            mail::mail_forget_invite,
            voice::voice_state,
            voice::voice_download,
            voice::voice_stop_download,
            voice::voice_remove,
            voice::voice_text,
            voice::voice_pieces,
            voice::voice_test_audio
        ]))
        .run(tauri::generate_context!())
        .expect("Xenify Chat failed to start");
}

/// The screens' calls: the core's (calendar, meetings, address books, out
/// of office) go to it by name; the rest to this app's own commands.
fn handler(
    own: impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    move |invoke| {
        use tauri::Manager;
        let app = invoke.message.webview().app_handle().clone();
        let chat = app.state::<host::Chat>();
        let args = match invoke.message.payload() {
            tauri::ipc::InvokeBody::Json(v) => v.clone(),
            _ => serde_json::Value::Null,
        };
        // Email kept by the server, when the app is signed in to one that does.
        if let Some(job) = servermail::intercept(&app, invoke.message.command(), &args) {
            let resolver = invoke.resolver;
            tauri::async_runtime::spawn(async move {
                match job.await {
                    Ok(Ok(v)) => resolver.resolve(v),
                    Ok(Err(e)) => resolver.reject(e),
                    Err(e) => resolver.reject(e.to_string()),
                }
            });
            return true;
        }
        match chat.call(invoke.message.command(), &args) {
            Some(Ok(v)) => {
                invoke.resolver.resolve(v);
                true
            }
            Some(Err(e)) => {
                invoke.resolver.reject(e);
                true
            }
            None => own(invoke),
        }
    }
}

/// Not signed in to a server: Xenify Hub signs every app in, so the app
/// opens it (or brings the running one forward) instead of asking itself.
#[tauri::command]
fn open_hub() -> Result<(), String> {
    xenify_hub_core::signin::open_hub()
}
