#![allow(dead_code)]
#![allow(unused_imports)]
use anyhow::{anyhow, Error};
use clap::{App, Arg, SubCommand};
use console::Style;
use dialoguer::{theme::ColorfulTheme, Confirmation, Input, Select};
use std::env;
use std::env::args;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

mod config;
mod kubectl;
mod terraform;

use config::Config;

fn main() -> Result<(), Error> {
    let default_dir = home_with(".config/clusterctl");
    let default_config = home_with(".config/clusterctl/config.toml");
    create_dir(&default_dir.clone()).expect("could not create default config dir");

    let app = App::new("clusterctl")
        .about("Interactive wrapper that stands up and tears down Kubernetes")
        .version("0.1.0")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .help("path to config.toml")
                .takes_value(true)
                .default_value(&default_config),
        )
        .subcommands(vec![
            SubCommand::with_name("cache-assets").about("cache generated kube configs locally"),
            SubCommand::with_name("destroy-cluster").about("destroy a k8s cluster"),
            SubCommand::with_name("destroy-kubernetes-ingress")
                .about("destroy the ingress DNS records"),
            SubCommand::with_name("launch-cluster")
                .about("launch a new k8s cluster with the terraform tectonic installer"),
            SubCommand::with_name("namespace-init")
                .about("create namespaces with secrets and config maps"),
            SubCommand::with_name("argo-init").about("install and configure argo on a cluster"),
            SubCommand::with_name("tool-check").about("check for required tools on PATH"),
        ]);

    // Load config
    let matches = app.get_matches();
    let config_path = matches
        .value_of("config")
        .ok_or(anyhow!("could not locate config"))?;
    let config = Config::from_file(&config_path)?;

    // Subcommands
    match matches.subcommand() {
        ("destroy-cluster", _) => destroy_cluster(&config)?,
        ("destroy-kubernetes-ingress", _) => destroy_kubernetes_ingress(&config, None)?,
        ("launch-cluster", _) => launch_cluster(&config)?,
        ("namespace-init", _) => namespace_init(&config, None)?,
        _ => return Err(anyhow!("you must provide a subcommand")),
    }

    Ok(())
}

fn namespace_init(conf: &Config, cluster_id: Option<String>) -> Result<(), Error> {
    let cluster_id = cluster_id.unwrap_or(pick_cluster_id_prompt()?);
    let infra_profile = &conf.infra_profile;

    // fetch kubeconfig
    let bucket = assets_bucket_name(&cluster_id, infra_profile)?;
    let cache_dir = Path::new(&conf.assets_cache_path).join(&cluster_id);
    create_dir(&cache_dir)?;
    let kubeconfig_path = cache_dir.join("kubeconfig");
    let path = kubeconfig_path
        .to_str()
        .ok_or(anyhow!("malformed assets path"))?;
    download_kubeconfig(&bucket, infra_profile, path)?;

    // create namespace
    let k = kubectl::Kubectl::new(&path);
    let d_ns = default_namespace(&cluster_id);

    if continue_prompt("Create default namespace?") {
        let status = k.create_namespace(d_ns)?;
        if !status.success() {
            return Err(anyhow!("could not create namespace"));
        }
    } else {
        return Ok(());
    }

    let secure_manifests = &conf.keybase_secure_manifests_path;
    let shared_secrets = Path::new(secure_manifests).join("secrets/shared");
    let default_ns_secrets = Path::new(secure_manifests).join(format!("secrets/{}", d_ns));
    let default_ns_configmaps = Path::new(secure_manifests).join(format!("configMaps/{}", d_ns));
    if continue_prompt("Deploy secrets and config maps?") {
        let status = k.create_with_manifest_recursive("kube-system", shared_secrets.to_str().unwrap())?;
        if !status.success() {
            println!("got an error creating kube-system secrets");
        }
        let status = k.create_with_manifest_recursive(d_ns, default_ns_secrets.to_str().unwrap())?;
        if !status.success() {
            println!("got an error creating {} namespace secrets", d_ns);
        }
        let status = k.create_with_manifest_recursive(d_ns, default_ns_configmaps.to_str().unwrap())?;
        if !status.success() {
            println!("got an error creating {} namespace configmaps", d_ns);
        }
        let status = k.create_config_map_literal("kube-system", "cluster-info", "cluster-name", &cluster_id)?;
        if !status.success() {
            println!("got an error creating cluster-info configmap in kube-system");
        }
        let status = k.create_config_map_literal(d_ns, "cluster-info", "cluster-name", &cluster_id)?;
        if !status.success() {
            println!("got an error creating cluster-info configmap in {}", d_ns);
        }
    } else {
        return Ok(());
    }

    Ok(())
}

fn launch_cluster(conf: &Config) -> Result<(), Error> {
    println!(
        r#"
This will step you through launching a cluster.
1. You will be prompted before each step
2. You will be shown the commands that will be run
3. STDOUT and STDERR will be printed to your console, as if you'd run the commands manually.
"#
    );
    let infra_profile = &conf.infra_profile;
    let cluster_id = pick_cluster_id_prompt()?;
    let path = Path::new(&conf.terraforming_path.clone()).join("projects/kubernetes-tectonic");
    println!("Path: {:?}", path);
    println!("Command: terraform get -update");
    if continue_prompt("Execute command?") {
        let status = terraform::get_update(&path)?;
        if !status.success() {
            return Err(anyhow!("could not update modules"));
        }
    } else {
        return Ok(());
    }

    println!("\nSelect the correct workspace");
    println!("Path: {:?}", path);
    println!("Command: terraform workspace select {}", cluster_id);
    if continue_prompt("Execute command?") {
        let status = terraform::workspace_select(&path, &cluster_id, &infra_profile)?;
        if !status.success() {
            return Err(anyhow!("select workspace {}", cluster_id));
        }
    } else {
        return Ok(());
    }

    println!("\nPlan changes to kubernetes-tectonic");
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_with_tfvars_file(&path, &cluster_id, &infra_profile)?;
        match status.code() {
            Some(1) => {
                return Err(anyhow!("could not plan kubernetes-tectonic"));
            }
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }

    println!("\nApply kubernetes-tectonic");
    println!("Path: {:?}", path);
    println!("Command: terraform apply tfplan.out");
    if continue_prompt("Execute command?") {
        let status = terraform::apply(&path, &infra_profile)?;
        if !status.success() {
            println!("\nAn error occurred, but this is expected");
        }
    } else {
        return Ok(());
    }

    println!("\nRe-plan changes to kubernetes-tectonic after expected error");
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_with_tfvars_file(&path, &cluster_id, &infra_profile)?;
        match status.code() {
            Some(1) => {
                return Err(anyhow!("could not re-plan kubernetes-tectonic"));
            }
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }
    println!("\nRe-apply kubernetes-tectonic");
    println!("Path: {:?}", path);
    println!("Command: terraform apply tfplan.out");
    if continue_prompt("Execute command?") {
        let status = terraform::apply(&path, &infra_profile)?;
        if !status.success() {
            return Err(anyhow!("unexpected error on second apply"));
        }
    } else {
        return Ok(());
    }

    println!("\nEnjoy your new cluster :)");

    Ok(())
}

fn destroy_kubernetes_ingress(conf: &Config, cluster_id: Option<String>) -> Result<(), Error> {
    let cluster_id = cluster_id.unwrap_or(pick_cluster_id_prompt()?);
    let path = Path::new(&conf.terraforming_path.clone()).join("projects/kubernetes-ingress");
    let v1_profile = &conf.v1_profile;

    println!("\nWe will now step through destroying the kubernetes-ingress project");
    println!("First, we must select the right workspace");
    println!("Path: {:?}", path);
    println!("Command: terraform workspace select {}", cluster_id);
    if continue_prompt("Execute command?") {
        let status = terraform::workspace_select(&path, &cluster_id, &v1_profile)?;
        if !status.success() {
            return Err(anyhow!("could not select workspace"));
        }
    } else {
        return Ok(());
    }

    println!(
        "\nWe will now prepare a -destroy plan against terraforming/projects/kubernetes-ingress"
    );
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars -destroy -detailed-exitcode",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_destroy_with_tfvars_file(&path, &cluster_id, v1_profile)?;
        match status.code() {
            Some(1) => return Err(anyhow!("unexpected error in -destroy plan")),
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }

    println!(
        "We are ready to apply. This will DESTROY DNS routes that point to {}",
        cluster_id
    );
    println!("Path: {:?}", path);
    println!("Command: terraform apply tfplan.out");
    if continue_prompt("Execute command?") {
        let status = terraform::apply(&path, v1_profile)?;
        if !status.success() {
            return Err(anyhow!("unexpected error"));
        }
    } else {
        return Ok(());
    }

    println!("\nWe have removed the DNS records!");

    let url = cluster_elbs_url(&cluster_id);
    println!(
        r#"
A manual step is required in the AWS web console.
There will be 3 ELBs created by Kubernetes that will be left running.

The following URL will show all ELBs related to cluster id {}

{}

INSPECT EACH ONE CAREFULLY BEFORE YOU DELETE IT. There should be 0 live instances
associated with the ELBs you delete.
"#,
        cluster_id, url
    );

    if continue_prompt("Open this url in your browser? (remember to use the the infra profile)") {
        open_browser(&url)?;
    }

    Ok(())
}

fn destroy_cluster(conf: &Config) -> Result<(), Error> {
    let theme = prompt_theme();

    println!(
        r#"
This will step you through destroying a cluster.
1. You will be prompted before each step
2. You will be shown the commands that will be run
3. STDOUT and STDERR will be printed to your console, as if you'd run the commands manually.
"#
    );
    if !continue_prompt("Do you want to proceed? (Use arrows)") {
        return Ok(());
    }
    println!("");

    let infra_profile = &conf.infra_profile;
    let cluster_id = pick_cluster_id_prompt()?;

    // destroy kubernetes-alarms
    // TODO

    // destroy kubernetes-tectonic
    let path = Path::new(&conf.terraforming_path.clone()).join("projects/kubernetes-tectonic");
    println!(
        "\nWe will now prepare a -destroy plan against terraforming/projects/kubernetes-tectonic"
    );
    println!("First, we must select the right workspace");
    println!("Path: {:?}", path);
    println!("Command: terraform workspace select {}", cluster_id);
    if !continue_prompt("Execute command?") {
        return Ok(());
    }

    let status = terraform::workspace_select(&path, &cluster_id, infra_profile)?;
    if !status.success() {
        return Err(anyhow!("terraform workspace select"));
    }

    println!("\nNext, we can optionally remove state that sometimes causes problems");
    println!("Path: {:?}", path);
    println!(
        r#"Command: terraform state rm \ 
    module.tectonic-aws.module.bootkube.template_dir.bootkube \
    module.tectonic-aws.module.tectonic.template_dir.tectonic \
    module.tectonic-aws.module.bootkube.template_dir.bootkube_bootstrap"#
    );

    let idx = Select::with_theme(&theme)
        .with_prompt("Execute command or skip?")
        .items(&["execute", "skip"])
        .interact()?;

    if idx == 0 {
        let status = terraform::state_rm(
            &path,
            &[
                "module.tectonic-aws.module.bootkube.template_dir.bootkube",
                "module.tectonic-aws.module.tectonic.template_dir.tectonic",
                "module.tectonic-aws.module.bootkube.template_dir.bootkube_bootstrap",
            ],
            infra_profile,
        )?;
        if !status.success() {
            return Err(anyhow!("error: terraform state rm"));
        }
    }

    println!("\nNext, we actually plan");
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars -destroy -detailed-exitcode",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_destroy_with_tfvars_file(&path, &cluster_id, infra_profile)?;
        // NOTE: we should be able to match on exit code 0 here to indicate no
        // diff was found, but it does not seem to work. We get exit code 2,
        // even when the plan shows no diff (e.g. -destroy against a cluster
        // that doesn't exist).
        match status.code() {
            Some(1) => {
                return Err(anyhow!(
                    r#"Could not "terraform plan". 
You probably need to re-run this tool and remove problematic bootkube/tectonic state."#
                ));
            }
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }

    println!("The plan we just ran should show approximately 120 resources to delete.");
    println!(
        r#"Unfortunately, the output of the plan does not contain the cluster id
so we must double check the workspace we are on!
"#
    );
    println!("Path: {:?}", path);
    println!("Command: terraform workspace show");
    if continue_prompt("Execute_command?") {
        println!("");
        let status = terraform::workspace_show(&path, infra_profile)?;
        if !status.success() {
            return Err(anyhow!("terraform workspace show"));
        }
    } else {
        return Ok(());
    }

    if !continue_prompt("\nAre we on the right workspace?") {
        return Err(anyhow!("wrong workspace"));
    }

    // destroy kubernetes-ingress
    println!("\nWe are ready to destroy the cluster. THERE IS NO GOING BACK");
    println!("Path: {:?}", path);
    println!("Command: terraform apply tfplan.out");
    if continue_prompt("Execute command?") {
        let status = terraform::apply(&path, infra_profile)?;
        if !status.success() {
            println!("\nterraform apply encountered an error, but this is expected.");
        }
    } else {
        return Ok(());
    }

    // TODO do we need TWO re-plan and re-applies?

    println!("We will now create another -destroy plan to ensure all resources are cleaned up");
    println!("This plan should show no diff");
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars -destroy -detailed-exitcode",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_destroy_with_tfvars_file(&path, &cluster_id, infra_profile)?;
        match status.code() {
            Some(1) => return Err(anyhow!("unexpected error")),
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }

    println!(
        "\nCluster destroy complete. DNS and ELBs associated with {} may still be up",
        cluster_id
    );
    if continue_prompt("Do you want to move on to destroying DNS and ELBs?") {
        destroy_kubernetes_ingress(conf, Some(cluster_id.to_owned()))?;
    }

    Ok(())
}

/// Join a path to the HOME directory. Panics on any error. HOME env var must be set.
fn home_with(path: &'static str) -> String {
    Path::new(&env::var("HOME").expect("HOME env var unset"))
        .join(path)
        .to_str()
        .unwrap()
        .to_owned()
}

/// Create a directory if it does not exist.
fn create_dir<P: AsRef<Path>>(p: P) -> Result<(), Error> {
    if !p.as_ref().exists() {
        std::fs::create_dir_all(p.as_ref())?;
    }
    Ok(())
}

fn valid_clusters() -> Vec<&'static str> {
    vec![
        "development0",
        "development1",
        "development2",
        "production0",
        "production1",
        "production2",
    ]
}

fn assets_bucket_name(cluster_id: &str, profile: &str) -> Result<String, Error> {
    let matcher = format!("a{}", cluster_id);
    use std::str::FromStr;
    let mut cmd = Command::new("aws");
    cmd.env("AWS_PROFILE", profile);
    cmd.args(vec![
        "s3api",
        "list-buckets",
        "--query",
        "Buckets[].Name",
        "--output",
        "text",
    ]);
    let output = cmd.output()?;
    if !output.status.success() {
        // println!("STDOUT {:?}", std::str::from_utf8(&output.stderr)?);
        // println!("STDERR {:?}", std::str::from_utf8(&output.stdout)?);
        return Err(anyhow!("listing buckets with aws cli failed"));
    }
    let s = std::str::from_utf8(&output.stdout)?;
    for item in s.split_whitespace() {
        if item.starts_with(&matcher) {
            return Ok(String::from_str(item)?);
        }
    }
    Err(anyhow!("could not locate assets bucket for {}", cluster_id))
}

fn download_kubeconfig(bucket: &str, profile: &str, output: &str) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("aws");
    cmd.env("AWS_PROFILE", profile);
    cmd.args(vec![
        "s3api",
        "get-object",
        "--bucket",
        bucket,
        "--key",
        "kubeconfig",
        output,
    ]);
    Ok(cmd.status()?)
}

fn continue_prompt(msg: &'static str) -> bool {
    let theme = prompt_theme();
    Select::with_theme(&theme)
        .with_prompt(msg)
        .items(&["yes", "no"])
        .interact()
        .unwrap()
        == 0
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

fn open_browser(url: &str) -> Result<(), Error> {
    // Conditional compilation: select the right open program for the OS.
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = Command::new("xdg-open");
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd.arg(url);
    cmd.spawn()?;
    Ok(())
}

fn cluster_elbs_url(cluster_id: &str) -> String {
    format!("https://console.aws.amazon.com/ec2/home?region=us-east-1#LoadBalancers:tag:kubernetes.io/cluster/{}=*", cluster_id)
}

fn pick_cluster_id_prompt() -> Result<String, Error> {
    let theme = prompt_theme();
    let ids = valid_clusters();
    let idx = Select::with_theme(&theme)
        .with_prompt("Select a cluster id")
        .items(&ids)
        .interact()?;
    Ok(ids[idx].to_owned())
}

fn default_namespace(cluster_id: &str) -> &'static str {
    if cluster_id.starts_with("development") {
        return "development";
    }
    if cluster_id.starts_with("production") {
        return "production";
    }
    unreachable!("unknown cluster id {}", cluster_id)
}
