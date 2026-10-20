//! Xenify Chat: the desktop app. The screens are HTML in `ui/`; this side talks
//! to the chat server and runs the spelling and grammar checker.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod calendar;
mod check;
mod mail;
mod net;
mod notify;
mod store;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    tauri::Builder::default()
        .manage(net::Net::new(&args))
        .manage(check::Check::new())
        .manage(store::Store::default())
        .manage(mail::Mail::default())
        .manage(calendar::Calendar::default())
        .invoke_handler(tauri::generate_handler![
            net::start,
            net::sign_in,
            net::send,
            net::sign_out,
            net::change_server,
            check::check,
            check::words,
            check::add_word,
            check::remove_word,
            check::spotter_state,
            check::checker_credits,
            notify::notify,
            notify::load_settings,
            notify::save_settings,
            store::search_messages,
            store::messages_between,
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
            calendar::cal_between,
            calendar::cal_due,
            calendar::cal_save,
            calendar::cal_delete,
            calendar::cal_reminded,
            calendar::cal_snoozed
        ])
        .run(tauri::generate_context!())
        .expect("Xenify Chat failed to start");
}
