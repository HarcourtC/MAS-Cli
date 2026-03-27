use std::env;
use std::io::{self, IsTerminal};

pub(crate) fn print_mas_cli_banner() {
    let lines = [
        " __  __    _    ____        ____ _ _ ",
        "|  \\/  |  / \\  / ___|      / ___| (_)",
        "| |\\/| | / _ \\ \\___ \\_____| |   | | |",
        "| |  | |/ ___ \\ ___) |_____| |___| | |",
        "|_|  |_/_/   \\_\\____/       \\____|_|_|",
    ];

    if should_use_color_output() {
        let bright = "\x1b[96m";
        let normal = "\x1b[36m";
        let dim = "\x1b[2m";
        let reset = "\x1b[0m";

        println!();
        println!("{}{}{}", dim, lines[0], reset);
        println!("{}{}{}", bright, lines[1], reset);
        println!("{}{}{}", normal, lines[2], reset);
        println!("{}{}{}", bright, lines[3], reset);
        println!("{}{}{}", dim, lines[4], reset);
        println!("{}MAS-Cli{}", dim, reset);
        println!();
    } else {
        println!();
        for line in lines {
            println!("{}", line);
        }
        println!("MAS-Cli");
        println!();
    }
}

fn should_use_color_output() -> bool {
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if cfg!(target_os = "windows") && env::var_os("TERM").is_none() {
        return false;
    }
    io::stdout().is_terminal()
}
