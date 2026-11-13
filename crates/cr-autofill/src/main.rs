//! `cr-autofill` — read a card over PC/SC and type a credential into the
//! focused field.

use std::time::Duration;

use cr_autofill::credential::{read_credential, Credential};
use cr_autofill::typer::{backend_is_authorized, default_typer};
use crlib::PcscContext;

type Result<T> = cr_autofill::Result<T>;

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("cr-autofill: {err}");
            1
        }
    };
    std::process::exit(code);
}

struct Opts {
    command: String,
    field: String,
    delay: Duration,
    reader: Option<String>,
}

fn run() -> Result<()> {
    let opts = parse_args()?;
    if cfg!(target_os = "macos") && !backend_is_authorized() {
        eprintln!(
            "note: cr-autofill needs Accessibility permission to type into other apps.\n\
             grant it in System Settings > Privacy & Security > Accessibility, then relaunch."
        );
    }
    match opts.command.as_str() {
        "fill" => fill(&opts),
        "watch" => watch(&opts),
        "diag" => diag(),
        other => Err(format!("unknown command {other}").into()),
    }
}

fn diag() -> Result<()> {
    use pcsc::{Context, Protocols, ReaderState, Scope, ShareMode, State};
    use std::ffi::CString;

    let ctx = Context::establish(Scope::User)?;
    let mut buf = [0u8; 4096];
    let names: Vec<String> = ctx
        .list_readers(&mut buf)?
        .map(|n| n.to_string_lossy().into_owned())
        .collect();

    if names.is_empty() {
        println!("no readers visible to this process");
        return Ok(());
    }

    for name in names {
        println!("reader: {name}");
        let cname = CString::new(name.clone())?;
        let mut states = [ReaderState::new(cname, State::UNAWARE)];
        match ctx.get_status_change(Some(Duration::from_secs(2)), &mut states) {
            Ok(()) | Err(pcsc::Error::Timeout) => {}
            Err(err) => {
                println!("  status error: {err}");
                continue;
            }
        }
        let es = states[0].event_state();
        println!(
            "  PRESENT={} MUTE={} EMPTY={} UNAVAILABLE={} INUSE={}",
            es.contains(State::PRESENT),
            es.contains(State::MUTE),
            es.contains(State::EMPTY),
            es.contains(State::UNAVAILABLE),
            es.contains(State::INUSE),
        );
        if !es.contains(State::PRESENT) {
            continue;
        }
        let cname = CString::new(name)?;
        match ctx.connect(&cname, ShareMode::Shared, Protocols::ANY) {
            Ok(card) => match card.status2_owned() {
                Ok(status) => println!(
                    "  ATR {}",
                    crlib::hexutil::encode_pretty(status.atr())
                ),
                Err(err) => println!("  connected, status error: {err}"),
            },
            Err(err) => println!("  connect error: {err}"),
        }
    }
    Ok(())
}

fn parse_args() -> Result<Opts> {
    let mut args = std::env::args().skip(1);
    let command = match args.next() {
        Some(c) => c,
        None => {
            eprintln!("{HELP}");
            return Err("missing command".into());
        }
    };
    let mut field = "pan".to_string();
    let mut delay = Duration::from_millis(500);
    let mut reader = None;
    while let Some(arg) = args.next() {
        if let Some(v) = arg.strip_prefix("--field=") {
            field = v.to_string();
        } else if let Some(v) = arg.strip_prefix("--delay=") {
            delay = Duration::from_millis(v.parse::<u64>()?);
        } else if let Some(v) = arg.strip_prefix("--reader=") {
            reader = Some(v.to_string());
        } else if arg == "-h" || arg == "--help" {
            eprintln!("{HELP}");
            return Err("help".into());
        } else {
            return Err(format!("unknown argument {arg}").into());
        }
    }
    Ok(Opts {
        command,
        field,
        delay,
        reader,
    })
}

fn fill(opts: &Opts) -> Result<()> {
    let (cred, _reader) = read_once(opts.reader.as_deref())?;
    type_field(&cred, &opts.field, opts.delay)
}

fn watch(opts: &Opts) -> Result<()> {
    let ctx = PcscContext::establish()?;
    let mut watch = ctx.card_watch();
    let mut filled = false;
    let mut mute_reported: Option<String> = None;
    loop {
        let snapshot = ctx.poll_cards_detailed(
            &mut watch,
            opts.reader.as_deref(),
            Duration::from_millis(400),
        )?;
        match snapshot.present {
            Some(name) if !filled => {
                mute_reported = None;
                eprintln!("card inserted in {name}");
                match read_credential_at(&ctx, &name) {
                    Ok(Some(cred)) => {
                        type_field(&cred, &opts.field, opts.delay)?;
                        filled = true;
                    }
                    Ok(None) => eprintln!("no payment credential on card"),
                    Err(err) => eprintln!("read failed: {err}"),
                }
            }
            Some(_) => {
                mute_reported = None;
            }
            None => {
                filled = false;
                let mute = snapshot
                    .statuses
                    .iter()
                    .find(|status| status.flags.present && status.flags.mute);
                match mute {
                    Some(status) if mute_reported.as_deref() != Some(&status.reader) => {
                        eprintln!(
                            "card in {} is present but MUTE: it is not answering ISO 7816 \
                             (T=0/T=1), so it cannot be read. Typical causes: a synchronous \
                             memory card (SLE 4442/4428), a contactless-only card on a contact \
                             reader, or dirty contacts.",
                            status.reader
                        );
                        mute_reported = Some(status.reader.clone());
                    }
                    Some(_) => {}
                    None => mute_reported = None,
                }
            }
        }
    }
}

fn read_once(reader: Option<&str>) -> Result<(Credential, String)> {
    let ctx = PcscContext::establish()?;
    if let Some(name) = mute_reader(&ctx, reader)? {
        return Err(mute_message(&name).into());
    }
    let mut card = ctx.connect(reader)?;
    let reader = card.reader_name().to_string();
    let cred = read_credential(&mut card)?
        .ok_or_else(|| "no payment credential on this card".to_string())?;
    Ok((cred, reader))
}

fn read_credential_at(
    ctx: &PcscContext,
    reader: &str,
) -> crlib::Result<Option<Credential>> {
    let mut card = ctx.connect(Some(reader))?;
    read_credential(&mut card)
}

fn mute_reader(
    ctx: &PcscContext,
    reader: Option<&str>,
) -> crlib::Result<Option<String>> {
    let mut watch = ctx.card_watch();
    let snapshot = ctx.poll_cards_detailed(
        &mut watch,
        reader,
        Duration::from_millis(50),
    )?;
    Ok(snapshot
        .statuses
        .iter()
        .find(|status| status.flags.present && status.flags.mute)
        .map(|status| status.reader.clone()))
}

fn mute_message(reader: &str) -> String {
    format!(
        "card in {reader} is present but MUTE: it is not answering ISO 7816 (T=0/T=1), \
         so it cannot be read. Typical causes: a synchronous memory card (SLE 4442/4428), \
         a contactless-only card on a contact reader, or dirty contacts."
    )
}

fn type_field(
    cred: &Credential,
    field: &str,
    delay: Duration,
) -> Result<()> {
    let text = cred
        .field(field)
        .ok_or_else(|| format!("unknown field {field:?} (pan|expiry|month|year|name)"))?;
    std::thread::sleep(delay);
    default_typer()?.type_text(&text)?;
    eprintln!("typed {field}");
    Ok(())
}

const HELP: &str = "\
cr-autofill — autofill a credential read from a smart card

USAGE:
    cr-autofill fill   [--field=FIELD] [--delay=MS] [--reader=NAME]
    cr-autofill watch  [--field=FIELD] [--delay=MS] [--reader=NAME]
    cr-autofill diag

FIELD: pan (default), expiry (MM/YY), month, year, name

`fill` reads the card once, waits DELAY so you can focus the target field,
then types the chosen value. `watch` waits for a card to be inserted, reads
it, and types on insertion; it re-arms when the card is removed.

`diag` prints each reader's raw PC/SC state (PRESENT/MUTE/EMPTY) and, when a
card is present, the ATR. Use it when a card is inserted but never appears:
a PRESENT+MUTE card is detected by the reader but not answering ISO 7816.
";
