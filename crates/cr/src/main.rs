//! List PC/SC readers, wait for a card, run the standard crlib registry,
//! and print the returned fields.
//!
//! This is the supported front-end. A GPUI view is a thin wrapper around
//! [`crlib::view_rows`]; see the README for a drop-in sketch.

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use crlib::{view_rows, PcscContext, Registry, RowKind};

fn main() -> ExitCode {
    if let Err(err) = run() {
        eprintln!("cr: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut wait = false;
    let mut timeout = Duration::from_secs(30);
    let mut reader: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-l" | "--list" => {
                list_readers()?;
                return Ok(());
            }
            "-w" | "--wait" => wait = true,
            "--timeout" => {
                let secs: u64 = args
                    .next()
                    .ok_or("`--timeout` needs a value in seconds")?
                    .parse()?;
                timeout = Duration::from_secs(secs);
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other}").into());
            }
            other => reader = Some(other.to_string()),
        }
    }

    let ctx = PcscContext::establish().map_err(hint_pcsc)?;
    let readers = ctx.readers().map_err(hint_pcsc)?;
    if readers.is_empty() {
        return Err("no PC/SC readers found (is pcscd running?)".into());
    }

    eprintln!("readers:");
    for name in &readers {
        eprintln!("  {name}");
    }

    let chosen = if wait {
        eprintln!("waiting for a card (timeout {}s)…", timeout.as_secs());
        ctx.wait_for_card(reader.as_deref(), timeout)?
    } else {
        match reader {
            Some(r) => r,
            None => readers.into_iter().next().unwrap(),
        }
    };

    eprintln!("connecting to {chosen}");
    let mut card = ctx.connect(Some(&chosen))?;
    eprintln!(
        "ATR {}",
        crlib::hexutil::encode_pretty(crlib::Transport::atr(&card))
    );

    let results = Registry::standard().probe(&mut card);
    if results.is_empty() {
        eprintln!("no built-in app recognized this card");
        return Ok(());
    }

    let infos: Vec<_> = results
        .into_iter()
        .filter_map(|p| match p.result {
            Ok(info) => Some(info),
            Err(err) => {
                eprintln!("[{}] error: {err}", p.app);
                None
            }
        })
        .collect();

    for row in view_rows(&infos) {
        match row.kind {
            RowKind::Heading => {
                println!();
                println!("== {} ==", row.label);
            }
            RowKind::Field => println!("  {:<22} {}", row.label, row.value),
            RowKind::Dump => println!("    {:<20} {}", row.label, row.value),
        }
    }
    println!();
    Ok(())
}

fn list_readers() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = PcscContext::establish().map_err(hint_pcsc)?;
    let readers = ctx.readers().map_err(hint_pcsc)?;
    if readers.is_empty() {
        println!("(no readers)");
    } else {
        for name in readers {
            println!("{name}");
        }
    }
    Ok(())
}

fn hint_pcsc(err: crlib::Error) -> Box<dyn std::error::Error> {
    let msg = format!(
        "{err}\n  hint: pcscd must be running and libpcsclite must be visible.\n  on NixOS: `services.pcscd.enable = true;` and open a `nix-shell` with `pcsclite pkg-config`."
    );
    msg.into()
}

fn print_help() {
    println!(
        "\
cr — inspect a smart card with the standard crlib registry

USAGE:
    cr [OPTIONS] [READER]

OPTIONS:
    -l, --list          list PC/SC readers and exit
    -w, --wait          wait until a card is present
        --timeout SECS  wait timeout (default 30)
    -h, --help          show this help

If READER is omitted, the first reader is used. A substring match is accepted.

The built-in EMV parser reports scheme, a masked PAN (last 4 only), expiry,
and the application label. Full card numbers, cardholder names, and CVVs are
never printed.
"
    );
}
