use crate::runner::Cmd;
use anyhow::{anyhow, Error};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

pub struct Kubectl<'a> {
    kubeconfig_path: &'a str,
}

impl<'a> Kubectl<'a> {
    pub fn new(p: &'a str) -> Self {
        Kubectl { kubeconfig_path: p }
    }

    pub fn apply(&self, ns: &str, manifest: &str) -> Result<ExitStatus, Error> {
        let mut cmd = Command::new("kubectl");
        cmd.env("KUBECONFIG", self.kubeconfig_path);
        cmd.args(vec!["apply", "-f", manifest]);
        Ok(cmd.status()?)
    }

    pub fn create_namespace(&self, ns: &str) -> Result<ExitStatus, Error> {
        let mut cmd = Command::new("kubectl");
        cmd.env("KUBECONFIG", self.kubeconfig_path);
        cmd.args(vec!["create", "ns", ns]);
        Ok(cmd.status()?)
    }

    pub fn create_with_manifest_recursive(&self, ns: &str, dir: &str) -> Result<ExitStatus, Error> {
        let mut cmd = Command::new("kubectl");
        cmd.env("KUBECONFIG", self.kubeconfig_path);
        cmd.args(vec!["create", "-n", ns, "-Rf", dir]);
        Ok(cmd.status()?)
    }

    pub fn create_config_map_literal(
        &self,
        ns: &str,
        cm: &str,
        key: &str,
        value: &str,
    ) -> Result<ExitStatus, Error> {
        let mut cmd = Command::new("kubectl");
        cmd.env("KUBECONFIG", self.kubeconfig_path);
        let arg = format!("--from-literal={}={}", key, value);
        cmd.args(vec!["create", "configmap", cm, &arg, "-n", ns]);
        Ok(cmd.status()?)
    }
}
