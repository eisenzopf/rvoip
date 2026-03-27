use colored::Colorize;
use tabled::{Table, Tabled};

pub fn print_table<T: Tabled>(data: &[T]) {
    if data.is_empty() {
        println!("{}", "No results".dimmed());
        return;
    }
    println!("{}", Table::new(data));
}

pub fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

pub fn print_success(msg: &str) {
    println!("{} {}", "OK".green().bold(), msg);
}

pub fn print_error(msg: &str) {
    eprintln!("{} {}", "ERROR".red().bold(), msg);
}

pub fn print_status_box(title: &str, items: &[(&str, &str)]) {
    let max_key_len = items.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let max_val_len = items.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let inner_width = (max_key_len + max_val_len + 4).max(title.len() + 4);

    let border = "=".repeat(inner_width);
    println!("+{}+", border);
    println!(
        "|{:^width$}|",
        format!(" {} ", title).bold(),
        width = inner_width
    );
    println!("+{}+", border);
    for (key, value) in items {
        println!(
            "| {:<kw$}: {:<vw$} |",
            key.dimmed(),
            value.bold(),
            kw = max_key_len,
            vw = inner_width - max_key_len - 4
        );
    }
    println!("+{}+", border);
}
