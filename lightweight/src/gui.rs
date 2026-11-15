//! egui window: account list on the left, details and actions on the right.
//! Built only with `--features gui`. Launching runs in the background so the
//! window keeps responding while the helper talks to Roblox.

use crate::helper;
use crate::store::{self, Account};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

pub struct App {
    path: PathBuf,
    accounts: Vec<Account>,
    selected: Option<usize>,
    new_name: String,
    new_note: String,
    new_cookie: String,
    place_id: String,
    helper_path: String,
    status: String,
    pending: Option<Receiver<String>>,
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
            helper_path: helper::default_helper_path().to_string_lossy().into_owned(),
            status,
            pending: None,
        }
    }

    fn persist(&mut self) {
        if let Err(e) = store::save(&self.path, &self.accounts) {
            self.status = format!("Save failed: {e}");
        }
    }

    fn add_account(&mut self) {
        let name = store::clean_field(&self.new_name);
        if name.is_empty() {
            self.status = "Enter an account name.".into();
            return;
        }
        if self
            .accounts
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(&name))
        {
            self.status = format!("'{name}' already exists.");
            return;
        }

        // Encrypt the cookie before anything is stored. The field is cleared either way.
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
        if self.pending.is_some() {
            self.status = "Still launching the previous game. Please wait.".into();
            return;
        }
        let Some(account) = self.selected.and_then(|i| self.accounts.get(i)) else {
            self.status = "Select an account first.".into();
            return;
        };
        if account.cookie_blob.is_empty() {
            self.status = format!(
                "{} has no saved cookie. Remove it and add it again with a cookie.",
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

        thread::spawn(move || {
            let message = match helper::launch(&helper_path, &blob, place_id) {
                Ok(()) => format!("Launched {name} in place {place_id}."),
                Err(e) => format!("Launch failed for {name}: {e}"),
            };
            let _ = tx.send(message);
        });

        self.pending = Some(rx);
        self.status = "Launching...".into();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pick up the result of a background launch.
        let finished = self.pending.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(message) = finished {
            self.status = message;
            self.pending = None;
        } else if self.pending.is_some() {
            ctx.request_repaint();
        }

        // Actions are collected here and applied after the panels are drawn.
        let mut add = false;
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
            if ui.button("Launch selected account").clicked() {
                launch = true;
            }

            ui.separator();
            ui.heading("Add account");
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("Account name"));
            ui.add(egui::TextEdit::singleline(&mut self.new_note).hint_text("Note (optional)"));
            ui.add(
                egui::TextEdit::singleline(&mut self.new_cookie)
                    .hint_text(".ROBLOSECURITY cookie (optional)")
                    .password(true),
            );
            if ui.button("Add account").clicked() {
                add = true;
            }

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
        if remove {
            self.remove_selected();
        }
        if launch {
            self.launch_selected();
        }
    }
}
