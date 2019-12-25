use anyhow::{anyhow, Error};
use console::Style;
use dialoguer::{theme::ColorfulTheme, Confirmation, Input, Select};
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::rc::Rc;

#[derive(Clone)]
pub struct Cmd<'a> {
    pub command: Vec<&'a str>,
    pub working_dir: Option<PathBuf>,
    pub env: Option<EnvVars>,
    pub writes_file: Option<PathBuf>,
}

type EnvVars = Rc<RefCell<HashMap<String, String>>>;

fn new_env_vars() -> EnvVars {
    Rc::new(RefCell::new(HashMap::new()))
}

impl<'a> Cmd<'a> {
    pub fn new(command: Vec<&'a str>) -> Cmd {
        Cmd {
            command: command,
            working_dir: None,
            env: Some(new_env_vars()),
            writes_file: None,
        }
    }

    pub fn env(&'a mut self, var: &'a str, value: &'a str) -> &'a mut Cmd {
        match &self.env {
            Some(e) => {
                let mut e = e.borrow_mut();
                e.insert(var.to_string(), value.to_string());
            }
            None => self.env = Some(new_env_vars()),
        }
        self
    }

    pub fn dir(&'a mut self, path: PathBuf) -> &'a mut Cmd {
        self.working_dir = Some(path);
        self
    }

    pub fn writes_file(&'a mut self, path: PathBuf) -> &'a mut Cmd {
        self.writes_file = Some(path);
        self
    }

    pub fn spawn(&self) -> Result<Child, Error> {
        if self.command.len() < 1 {
            return Err(anyhow!("invalid command"));
        }

        let mut c = Command::new(self.command[0]);
        if self.command.len() > 1 {
            for arg in self.command[1..].iter() {
                c.arg(arg);
            }
        }
        if let Some(env_vars) = &self.env {
            for (ref k, ref v) in env_vars.borrow().iter() {
                c.env(k, v);
            }
        }
        if let Some(cwd) = &self.working_dir {
            c.current_dir(&cwd);
        }

        Ok(c.spawn()?)
    }
}

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

#[macro_export]
macro_rules! prompt_run {
    // prompt; cmd->Proc; Expect; on failure say
    ($prompt:literal, $cmd:expr, $expect:expr) => {{
        use crate::runner::{Expect, Proc};
        use std::env::current_dir;
        use std::process::Child;

        // print message
        // print path
        println!(
            "PATH: {:?}",
            $cmd.working_dir.as_ref().unwrap_or(&current_dir()?)
        );
        // print command
        println!(
            "COMMAND: {:?}",
            $cmd.command

        );
        // print choice prompt

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
                // spawn - get child
                let mut child = $cmd.spawn()?;
                let status = child.wait()?;
                let ok = {
                    match $expect {
                        Expect::Success => status.success(),
                        Expect::Failure => !status.success(),
                        Expect::Code(code) => status.code().unwrap() == code,
                        Expect::Any => true,
                        _ => unimplemented!(),
                    }
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
            // we know only 0, 1, 2 are reachable
            _ => unreachable!(),
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

#[test]
fn test_cmd() {
    use std::path::Path;
    let p = Path::new("/usr/local/bin");
    let mut c = Cmd::new(vec!["ls", "-l", p.to_str().unwrap()]);
    let c = c.env("KEY", "VALUE");

    let e = &c.env;
    let e = e.as_ref().unwrap();
    assert_eq!(
        *e.borrow().get(&"KEY".to_string()).unwrap(),
        "VALUE".to_string(),
    );
}
