//! egui window: account list on the left, details and actions on the right.
//! Built only with `--features gui`. Launching and browser login run in the
//! background so the window keeps responding.

use crate::helper;
use crate::store::{self, Account};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// A browser login in progress: the account name it's for, and where the result will arrive.
struct PendingLogin {
    name: String,
    note: String,
    result: Receiver<Result<String, String>>,
}

pub struct App {
    path: PathBuf,
    accounts: Vec<Account>,
    selected: Option<usize>,
    new_name: String,
    new_note: String,
    new_cookie: String,
    place_id: String,
    job_id: String,
    helper_path: String,
    status: String,
    pending_launch: Option<Receiver<String>>,
    pending_login: Option<PendingLogin>,
}

impl App {
    pub fn new(path: PathBuf) -> Self {
        let (accounts, status) = match store::load(&path) {
            Ok(a) => (a, String::new()),
            Err(e) => (Vec::new(), format!("Could not load accounts: {e}")),
        };
        Self {
            path,
            accounts,
            selected: None,
            new_name: String::new(),
            new_note: String::new(),
            new_cookie: String::new(),
            place_id: String::new(),
            job_id: String::new(),
            helper_path: helper::default_helper_path().to_string_lossy().into_owned(),
            status,
            pending_launch: None,
            pending_login: None,
        }
    }

    fn persist(&mut self) {
        if let Err(e) = store::save(&self.path, &self.accounts) {
            self.status = format!("Save failed: {e}");
        }
    }

    fn name_is_free(&self, name: &str) -> bool {
        !self
            .accounts
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name))
    }

    fn add_account(&mut self) {
        let name = store::clean_field(&self.new_name);
        if name.is_empty() {
            self.status = "Enter an account name.".into();
            return;
        }
        if !self.name_is_free(&name) {
            self.status = format!("'{name}' already exists.");
            return;
        }

        // Encrypt the pasted cookie before anything is stored. The field is cleared either way.
        let cookie = std::mem::take(&mut self.new_cookie);
        let cookie_blob = if cookie.trim().is_empty() {
            String::new()
        } else {
            match helper::protect(&PathBuf::from(&self.helper_path), &cookie) {
                Ok(blob) => blob,
                Err(e) => {
                    self.status = format!("Could not save cookie: {e}");
                    return;
                }
            }
        };

        let note = store::clean_field(&self.new_note);
        self.accounts.push(Account {
            name: name.clone(),
            note,
            cookie_blob,
        });
        self.selected = Some(self.accounts.len() - 1);
        self.new_name.clear();
        self.new_note.clear();
        self.persist();
        self.status = format!("Added {name}.");
    }

    fn start_browser_login(&mut self) {
        if self.pending_login.is_some() {
            self.status = "A login window is already open.".into();
            return;
        }
        let name = store::clean_field(&self.new_name);
        if name.is_empty() {
            self.status = "Enter an account name first, then log in.".into();
            return;
        }
        if !self.name_is_free(&name) {
            self.status = format!("'{name}' already exists.");
            return;
        }

        let (tx, rx) = mpsc::channel();
        let helper_path = PathBuf::from(&self.helper_path);
        thread::spawn(move || {
            let _ = tx.send(helper::login(&helper_path));
        });

        self.pending_login = Some(PendingLogin {
            name: name.clone(),
            note: store::clean_field(&self.new_note),
            result: rx,
        });
        self.status = format!("Log in to Roblox in the window for {name}...");
    }

    fn remove_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            if i < self.accounts.len() {
                let removed = self.accounts.remove(i);
                self.persist();
                self.status = format!("Removed {}.", removed.name);
            }
        }
    }

    fn launch_selected(&mut self) {
        if self.pending_launch.is_some() {
            self.status = "Still launching the previous game. Please wait.".into();
            return;
        }
        let Some(account) = self.selected.and_then(|i| self.accounts.get(i)) else {
            self.status = "Select an account first.".into();
            return;
        };
        if account.cookie_blob.is_empty() {
            self.status = format!(
                "{} has no saved cookie. Log in again or add a cookie.",
                account.name
            );
            return;
        }
        let place_id = match self.place_id.trim().parse::<u64>() {
            Ok(id) => id,
            Err(_) => {
                self.status = "Place ID must be a number.".into();
                return;
            }
        };

        let (tx, rx) = mpsc::channel();
        let helper_path = PathBuf::from(&self.helper_path);
        let blob = account.cookie_blob.clone();
        let name = account.name.clone();
        let job_id = self.job_id.trim().to_string();
        let joining = if job_id.is_empty() {
            "any server"
        } else {
            "job "
        };

        let thread_name = name.clone();
        thread::spawn(move || {
            let message = match helper::launch(&helper_path, &blob, place_id, &job_id) {
                Ok(()) => format!("Launched {thread_name} in place {place_id}."),
                Err(e) => format!("Launch failed for {thread_name}: {e}"),
            };
            let _ = tx.send(message);
        });

        self.pending_launch = Some(rx);
        self.status = format!("Launching {name} into {joining}...");
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pick up the result of a background launch.
        let launch_done = self
            .pending_launch
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some(message) = launch_done {
            self.status = message;
            self.pending_launch = None;
        }

        // Pick up the result of a browser login: save the account if it worked.
        let login_done = self
            .pending_login
            .as_ref()
            .and_then(|p| p.result.try_recv().ok());
        if let Some(result) = login_done {
            if let Some(login) = self.pending_login.take() {
                match result {
                    Ok(blob) if !blob.is_empty() => {
                        self.accounts.push(Account {
                            name: login.name.clone(),
                            note: login.note,
                            cookie_blob: blob,
                        });
                        self.selected = Some(self.accounts.len() - 1);
                        self.new_name.clear();
                        self.new_note.clear();
                        self.persist();
                        self.status = format!("Logged in and saved {}.", login.name);
                    }
                    Ok(_) => self.status = "Login did not return a cookie.".into(),
                    Err(e) => self.status = format!("Login failed: {e}"),
                }
            }
        }

        if self.pending_launch.is_some() || self.pending_login.is_some() {
            ctx.request_repaint();
        }

        // Actions are collected here and applied after the panels are drawn.
        let mut add = false;
        let mut login = false;
        let mut remove = false;
        let mut launch = false;
        let mut note_changed = false;

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label(self.status.as_str());
        });

        egui::SidePanel::left("accounts")
            .resizable(true)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.heading("Accounts");
                ui.label(format!("{} saved", self.accounts.len()));
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, account) in self.accounts.iter().enumerate() {
                        let label = if account.note.is_empty() {
                            account.name.clone()
                        } else {
                            format!("{}  -  {}", account.name, account.note)
                        };
                        if ui
                            .selectable_label(self.selected == Some(i), label)
                            .clicked()
                        {
                            self.selected = Some(i);
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Selected account");
            match self.selected.and_then(|i| self.accounts.get_mut(i)) {
                Some(account) => {
                    ui.label(format!("Name: {}", account.name));
                    let cookie_status = if account.cookie_blob.is_empty() {
                        "none (launching needs a cookie)"
                    } else {
                        "saved (encrypted)"
                    };
                    ui.label(format!("Cookie: {cookie_status}"));
                    ui.horizontal(|ui| {
                        ui.label("Note:");
                        if ui.text_edit_singleline(&mut account.note).changed() {
                            note_changed = true;
                        }
                    });
                    if ui.button("Remove account").clicked() {
                        remove = true;
                    }
                }
                None => {
                    ui.label("Select an account on the left, or add one below.");
                }
            }

            ui.separator();
            ui.heading("Launch");
            ui.add(egui::TextEdit::singleline(&mut self.place_id).hint_text("Place ID"));
            ui.add(
                egui::TextEdit::singleline(&mut self.job_id)
                    .hint_text("Job ID (optional, joins that exact server)"),
            );
            if ui.button("Launch selected account").clicked() {
                launch = true;
            }

            ui.separator();
            ui.heading("Add account");
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("Account name"));
            ui.add(egui::TextEdit::singleline(&mut self.new_note).hint_text("Note (optional)"));
            ui.horizontal(|ui| {
                if ui.button("Log in with browser").clicked() {
                    login = true;
                }
                if ui.button("Add with pasted cookie").clicked() {
                    add = true;
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.new_cookie)
                    .hint_text("Or paste a .ROBLOSECURITY cookie")
                    .password(true),
            );

            ui.separator();
            ui.collapsing("Helper settings", |ui| {
                ui.label("Path to RobloxProcessLookup.exe:");
                ui.text_edit_singleline(&mut self.helper_path);
            });
        });

        if note_changed {
            self.persist();
        }
        if add {
            self.add_account();
        }
        if login {
            self.start_browser_login();
        }
        if remove {
            self.remove_selected();
        }
        if launch {
            self.launch_selected();
        }
    }
}
