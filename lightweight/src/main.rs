//! Lightweight Roblox alt-account list.
//!
//! Build the window:      cargo run --release --features gui
//! Command-line only:     cargo run -- --cli   (or build without the gui feature)

mod helper;
mod store;

#[cfg(feature = "gui")]
mod gui;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use store::{Account, clean_field, config_path, load, open_place, save};

fn main() {
    let path = config_path();

    #[cfg(feature = "gui")]
    {
        if !std::env::args().any(|a| a == "--cli") {
            run_gui(path);
            return;
        }
    }

    run_cli(path);
}

#[cfg(feature = "gui")]
fn run_gui(path: PathBuf) {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Roblox Alt Manager",
        options,
        Box::new(move |_cc: &eframe::CreationContext<'_>| -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Box::new(gui::App::new(path.clone())))
        }),
    );
    if let Err(e) = result {
        eprintln!("Window error: {e}");
    }
}

fn run_cli(path: PathBuf) {
    let mut accounts = match load(&path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Could not read {}: {e}", path.display());
            std::process::exit(1);
        }
    };

    println!("Roblox Alt Manager (command line). Type 'help' for commands.");
    let stdin = io::stdin();
    loop {
        print!("> ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("input error: {e}");
                break;
            }
        }
        match run_command(line.trim(), &mut accounts, &path) {
            Ok(true) => {}
            Ok(false) => break,
            Err(msg) => eprintln!("error: {msg}"),
        }
    }
}

fn print_help() {
    println!(
        "Commands:
  list                     Show all accounts
  add <name> [note...]     Add an account
  remove <name>            Remove an account
  note <name> <text...>    Change an account's note
  open <place_id>          Open a Roblox place in the default-signed-in client
  launch <name> <place_id> [job_id]  Launch a place for an account that has a saved cookie
  help                     Show this help
  quit                     Exit"
    );
}

/// Returns Ok(false) when the user asked to quit.
fn run_command(line: &str, accounts: &mut Vec<Account>, path: &PathBuf) -> Result<bool, String> {
    let mut words = line.split_whitespace();
    let Some(cmd) = words.next() else {
        return Ok(true);
    };
    let rest: Vec<&str> = words.collect();

    match cmd {
        "list" | "ls" => {
            if accounts.is_empty() {
                println!("No accounts yet. Use: add <name> [note]");
            }
            for (i, a) in accounts.iter().enumerate() {
                println!("{:>3}. {:<24} {}", i + 1, a.name, a.note);
            }
        }
        "add" => {
            let name = rest.first().ok_or("usage: add <name> [note]")?.to_string();
            if accounts.iter().any(|a| a.name.eq_ignore_ascii_case(&name)) {
                return Err(format!("'{name}' already exists"));
            }
            let note = clean_field(&rest[1..].join(" "));
            accounts.push(Account {
                name: name.clone(),
                note,
                cookie_blob: String::new(),
            });
            save(path, accounts).map_err(|e| e.to_string())?;
            println!("Added {name}");
        }
        "remove" | "rm" => {
            let name = rest.first().ok_or("usage: remove <name>")?;
            let before = accounts.len();
            accounts.retain(|a| !a.name.eq_ignore_ascii_case(name));
            if accounts.len() == before {
                return Err(format!("no account named '{name}'"));
            }
            save(path, accounts).map_err(|e| e.to_string())?;
            println!("Removed {name}");
        }
        "note" => {
            let name = rest.first().ok_or("usage: note <name> <text>")?;
            let account = accounts
                .iter_mut()
                .find(|a| a.name.eq_ignore_ascii_case(name))
                .ok_or(format!("no account named '{name}'"))?;
            account.note = rest[1..].join(" ");
            save(path, accounts).map_err(|e| e.to_string())?;
            println!("Updated note for {name}");
        }
        "open" => {
            let id: u64 = rest
                .first()
                .ok_or("usage: open <place_id>")?
                .parse()
                .map_err(|_| "place id must be a number".to_string())?;
            open_place(id).map_err(|e| e.to_string())?;
            println!("Opened place {id}");
        }
        "launch" => {
            let name = rest.first().ok_or("usage: launch <name> <place_id>")?;
            let id: u64 = rest
                .get(1)
                .ok_or("usage: launch <name> <place_id>")?
                .parse()
                .map_err(|_| "place id must be a number".to_string())?;
            let account = accounts
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(name))
                .ok_or(format!("no account named '{name}'"))?;
            if account.cookie_blob.is_empty() {
                return Err(format!("'{name}' has no saved cookie"));
            }
            let job = rest.get(2).copied().unwrap_or("");
            helper::launch(
                &helper::default_helper_path(),
                &account.cookie_blob,
                id,
                job,
            )?;
            println!("Launched {name} in place {id}");
        }
        "help" | "?" => print_help(),
        "quit" | "exit" | "q" => return Ok(false),
        other => return Err(format!("unknown command '{other}' (try 'help')")),
    }
    Ok(true)
}
