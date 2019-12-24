use anyhow::{anyhow, Error};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

const ARGO_TEMPLATE: &'static str = "/tmp/argo_template.yaml";

pub fn dep_update(chart_path: &str) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("helm");
    cmd.args(vec!["dep", "update", chart_path]);
    Ok(cmd.status()?)
}

pub fn template_argocd(chart_path: &str, values_file: &str) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("helm");
    cmd.args(vec!["template", "-f", values_file, chart_path]);
    let output = cmd.output()?;
    let mut f = std::fs::File::create(ARGO_TEMPLATE)?;
    f.write_all(&output.stdout)?;
    Ok(output.status)
}
