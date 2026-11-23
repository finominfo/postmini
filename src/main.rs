#![cfg_attr(all(windows, not(debug_assertions), not(feature = "console")), windows_subsystem = "windows")]

mod auth;
mod cli;
mod codegen;
mod http;
mod importer;
mod mock;
mod model;
mod prepare;
mod runner;
mod script;
mod settings;
mod storage;
mod ui;
mod vars;
mod ws;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() && !args[0].starts_with("--open") {
        #[cfg(windows)]
        unsafe {
            // GUI subsystem binary: attach to the parent console so CLI output is visible
            windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
        let code = cli::main(args);
        std::process::exit(code);
    }
    if let Err(e) = ui::run() {
        eprintln!("postmini: {e}");
        std::process::exit(1);
    }
}
