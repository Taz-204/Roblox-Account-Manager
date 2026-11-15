//! egui window: account list on the left, details and actions on the right.
//! Built only with `--features gui`.

use crate::store::{self, Account};
use eframe::egui;
use std::path::PathBuf;

pub struct App {
    path: PathBuf,
    accounts: Vec<Account>,
    selected: Option<usize>,
    new_name: String,
    new_note: String,
    place_id: String,
    status: String,
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
            place_id: String::new(),
            status,
        }
    }

    fn persist(&mut self) {
        if let Err(e) = store::save(&self.path, &self.accounts) {
            self.status = format!("Save failed: {e}");
        }
    }

    fn add_account(&mut self) {
        let name = self.new_name.trim().to_string();
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
        let note = self.new_note.trim().to_string();
        self.accounts.push(Account {
            name: name.clone(),
            note,
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

    fn launch_place(&mut self) {
        match self.place_id.trim().parse::<u64>() {
            Ok(id) => {
                self.status = match store::open_place(id) {
                    Ok(()) => format!("Opened place {id}."),
                    Err(e) => format!("Could not open place: {e}"),
                };
            }
            Err(_) => self.status = "Place ID must be a number.".into(),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Actions are collected here and applied after the panels are drawn,
        // so the panel closures never need to borrow all of `self` at once.
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
            ui.heading("Add account");
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("Account name"));
            ui.add(egui::TextEdit::singleline(&mut self.new_note).hint_text("Note (optional)"));
            if ui.button("Add account").clicked() {
                add = true;
            }

            ui.separator();
            ui.heading("Open a place");
            ui.add(egui::TextEdit::singleline(&mut self.place_id).hint_text("Place ID"));
            if ui.button("Open in Roblox").clicked() {
                launch = true;
            }
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
            self.launch_place();
        }
    }
}
