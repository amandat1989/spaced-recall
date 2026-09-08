use std::env;
use std::process::ExitCode;

use spaced_recall::{Card, Scheduler};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    let Some(command) = args.first() else {
        print_usage();
        return ExitCode::FAILURE;
    };

    match command.as_str() {
        "new" => run_new(&args[1..]),
        "review" => run_review(&args[1..]),
        "help" | "-h" | "--help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown command: {other}");
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  srs new --today <day>");
    eprintln!(
        "  srs review --interval <days> --reps <n> --ease <factor> --due <day> --today <day> --grade <0-5> [--lenient]"
    );
}

fn run_new(args: &[String]) -> ExitCode {
    let flags = match parse_flags(args) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    let today = match flags.get_u32("today") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };

    print_card(&Card::new(today));
    ExitCode::SUCCESS
}

fn run_review(args: &[String]) -> ExitCode {
    let flags = match parse_flags(args) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };

    let interval_days = match flags.get_u32("interval") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let repetitions = match flags.get_u32("reps") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let ease = match flags.get_f64("ease") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let due_on = match flags.get_u32("due") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let today = match flags.get_u32("today") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let grade = match flags.get_u8("grade") {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };

    let card = Card {
        interval_days,
        repetitions,
        ease,
        due_on,
    };
    let scheduler = if flags.has_switch("lenient") {
        Scheduler::lenient()
    } else {
        Scheduler::strict()
    };

    match scheduler.review(&card, grade, today) {
        Ok(next) => {
            print_card(&next);
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn print_card(card: &Card) {
    println!(
        "interval={} reps={} ease={:.2} due={}",
        card.interval_days, card.repetitions, card.ease, card.due_on
    );
}

fn fail(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::FAILURE
}

/// A hand-rolled `--flag value` parser. Good enough for a handful of flags
/// and one switch; not meant to grow into a general option parser.
struct Flags {
    values: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Flags {
    fn get_str(&self, name: &str) -> Result<&str, String> {
        self.values
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| format!("missing required flag --{name}"))
    }

    fn get_u32(&self, name: &str) -> Result<u32, String> {
        self.get_str(name)?
            .parse()
            .map_err(|_| format!("--{name} must be a whole number"))
    }

    fn get_u8(&self, name: &str) -> Result<u8, String> {
        self.get_str(name)?
            .parse()
            .map_err(|_| format!("--{name} must be a number from 0 to 255"))
    }

    fn get_f64(&self, name: &str) -> Result<f64, String> {
        self.get_str(name)?
            .parse()
            .map_err(|_| format!("--{name} must be a decimal number"))
    }

    fn has_switch(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

fn parse_flags(args: &[String]) -> Result<Flags, String> {
    let mut values = Vec::new();
    let mut switches = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        let Some(name) = arg.strip_prefix("--") else {
            return Err(format!("unexpected argument: {arg}"));
        };

        if name == "lenient" {
            switches.push(name.to_string());
            i += 1;
            continue;
        }

        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("--{name} needs a value"))?;
        values.push((name.to_string(), value.clone()));
        i += 2;
    }

    Ok(Flags { values, switches })
}
