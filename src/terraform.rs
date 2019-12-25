use crate::runner::Proc;
use anyhow::{anyhow, Error};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

pub fn plan_destroy_with_tfvars_file<P: AsRef<Path>>(
    dir: P,
    tfvars: &str,
    profile: &str,
) -> Result<ExitStatus, Error> {
    let tfvars = format!("{}.tfvars", tfvars);
    let mut cmd = Command::new("terraform");
    cmd.env("AWS_PROFILE", profile);
    cmd.current_dir(&dir);
    cmd.args(vec![
        "plan",
        "-out",
        "tfplan.out",
        "-var-file",
        &tfvars,
        "-destroy",
        "-detailed-exitcode",
    ]);
    Ok(cmd.status()?)
}

pub fn plan_with_tfvars_file<P: AsRef<Path>>(
    dir: P,
    tfvars: &str,
    profile: &str,
) -> Result<ExitStatus, Error> {
    let tfvars = format!("{}.tfvars", tfvars);
    let mut cmd = Command::new("terraform");
    cmd.env("AWS_PROFILE", profile);
    cmd.current_dir(&dir);
    cmd.args(vec![
        "plan",
        "-out",
        "tfplan.out",
        "-var-file",
        &tfvars,
        "-detailed-exitcode",
    ]);
    Ok(cmd.status()?)
}

pub fn apply<P: AsRef<Path>>(dir: P, profile: &str) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("terraform");
    cmd.env("AWS_PROFILE", profile);
    cmd.current_dir(&dir);
    cmd.args(vec!["apply", "tfplan.out"]);
    Ok(cmd.status()?)
}

pub fn workspace_select<P: AsRef<Path>>(
    dir: P,
    workspace: &str,
    profile: &str,
) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("terraform");
    cmd.env("AWS_PROFILE", profile);
    cmd.current_dir(&dir);
    cmd.args(vec!["workspace", "select", workspace]);
    Ok(cmd.status()?)
}

pub fn workspace_show<P: AsRef<Path>>(dir: P, profile: &str) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("terraform");
    cmd.env("AWS_PROFILE", profile);
    cmd.current_dir(&dir);
    cmd.args(vec!["workspace", "show"]);
    Ok(cmd.status()?)
}

pub fn version<P: AsRef<Path>>(dir: P, profile: &str) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("terraform");
    cmd.env("AWS_PROFILE", profile);
    cmd.current_dir(&dir);
    cmd.args(vec!["version"]);
    Ok(cmd.status()?)
}

pub fn state_rm<P: AsRef<Path>>(
    dir: P,
    states: &[&str],
    profile: &str,
) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("terraform");
    cmd.env("AWS_PROFILE", profile);
    cmd.current_dir(&dir);
    cmd.args(vec!["state", "rm"]);
    for state in states {
        cmd.arg(state);
    }
    Ok(cmd.status()?)
}

pub fn get_update<P: AsRef<Path>>(dir: P) -> Result<Proc, Error> {
    let mut cmd = Command::new("terraform");
    cmd.current_dir(&dir);
    cmd.args(vec!["get", "-update"]);
    Ok(Proc::Status(cmd.status()?))
}
