use console::Style;
use dialoguer::{theme::ColorfulTheme, Confirmation, Input, Select};
use std::process::{Child, ExitStatus};

pub enum Proc {
    Child(Child),
    Status(ExitStatus),
}

pub enum Expect<'a> {
    Code(i32),
    Output(&'a str),
    CodeWithOutput((i32, &'a str)),
    SuccessWithOutput(&'a str),
    FailureWithOutput(&'a str),
    Success,
    Failure,
    Any,
}

fn runtime_assert(p: Proc, e: Expect) {
    // match invalid cases, otherwise pass through
    let expects_output = match e {
        Expect::CodeWithOutput(_)
        | Expect::SuccessWithOutput(_)
        | Expect::FailureWithOutput(_)
        | Expect::Output(_) => true,
        _ => false,
    };
    match p {
        Proc::Status(_) if expects_output => {
            panic!("commands that expect output cannot return Proc::Status");
        }
        _ => { /* pass */ }
    }
}

#[macro_export]
macro_rules! prompt_run {
    // prompt; cmd->Proc; Expect; on failure say
    ($prompt:literal; $cmd:expr; $expect:expr) => {{
        runtime_assert($cmd, $expect);

        let theme = prompt_theme();
        let idx = Select::with_theme(&theme)
            .with_prompt($prompt)
            .items(&["execute", "exit", "skip"])
            .interact()
            .unwrap();

        match idx {
            2 => { /* skip */ }
            1 => std::process::exit(1),
            0 => {
                // execute
                let p: Child = $cmd?;
                let ok = match $expect {
                    Expect::Success => p.wait().success(),
                    Expect::Failure => !p.wait().success(),
                    Expect::Code(code) => p.wait().code == code,
                    Expect::Any => true,
                    _ => unimplemented!(),
                };

                if !ok {
                    let idx = Select::with_theme(&theme)
                        .with_prompt("Previous command behaved unexpectedly. Proceed with caution.")
                        .items(&["continue", "exit"])
                        .interact()
                        .unwrap();
                    if idx == 1 {
                        std::process::exit(1)
                    }
                }
            }
        }
    }};
}

fn prompt_theme() -> ColorfulTheme {
    ColorfulTheme {
        values_style: Style::new().yellow().dim(),
        indicator_style: Style::new().yellow().bold(),
        yes_style: Style::new().yellow().dim(),
        no_style: Style::new().yellow().dim(),
        ..ColorfulTheme::default()
    }
}
