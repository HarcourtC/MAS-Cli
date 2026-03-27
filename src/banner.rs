use std::env;
use std::io::{self, IsTerminal};

pub(crate) fn print_mas_cli_banner() {
    let lines = [
        " █████  ██    ██ ████████  ██████      ███    ███  █████   ██████ ",
        "██   ██ ██    ██    ██    ██    ██     ████  ████ ██   ██ ██      ",
        "███████ ██    ██    ██    ██    ██     ██ ████ ██ ███████  █████  ",
        "██   ██ ██    ██    ██    ██    ██     ██  ██  ██ ██   ██      ██ ",
        "██   ██  ██████     ██     ██████      ██      ██ ██   ██ ██████  ",
    ];

    if should_use_color_output() {
        let grad = [
            "\x1b[38;5;39m",
            "\x1b[38;5;45m",
            "\x1b[38;5;99m",
            "\x1b[38;5;141m",
            "\x1b[38;5;177m",
            "\x1b[38;5;204m",
        ];
        let muted = "\x1b[38;5;246m";
        let reset = "\x1b[0m";

        println!();
        for (line, color) in lines.iter().zip(grad.iter()) {
            println!("{}{}{}", color, line, reset);
        }
        println!();
        println!("{}Tips for getting started:{}", muted, reset);
        println!("{}1. Run backend status/start/stop{}", muted, reset);
        println!(
            "{}2. Run queue list or queue start --queue-id <id>{}",
            muted, reset
        );
        println!("{}3. Type help for commands, exit to quit{}", muted, reset);
        println!();
    } else {
        println!();
        for line in lines {
            println!("{}", line);
        }
        println!();
        println!("Tips for getting started:");
        println!("1. Run backend status/start/stop");
        println!("2. Run queue list or queue start --queue-id <id>");
        println!("3. Type help for commands, exit to quit");
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
